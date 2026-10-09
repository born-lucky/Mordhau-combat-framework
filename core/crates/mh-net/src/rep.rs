//! The replication half of one character's UMotionSystemComponent on one machine, plus the character fields it owns
//! (AMordhauCharacter ReplicatedNetMotion, ReplicatedHealth, ReplicatedStamina, NetBlock, ...). Port of
//! godot/game/net/net_motion_rep.gd. Sources: extract/native/decomp/UMotionSystemComponent.cpp and
//! AMordhauCharacter.cpp; field offsets from extract/native/types/UMotionSystemComponent.h and AMordhauCharacter.h.
//!
//! The game's model, as the code runs it:
//!   - Every motion change that goes over the network is an FNetMotion (Id, MotionType, Param0..2, DynamicParam).
//!     AssignNetMotion gives it Id = NetMotion.Id + 1 on every machine.
//!   - The owning client predicts: it runs the motion at once (HandleNetMotionUpdate with bIsLocalSimulation) and
//!     sends ServerAssignNetMotion(motion, LastAuthoritativeMotionID). The server accepts it only when the Id is the
//!     next one.
//!   - The server's own motion changes (a parry's Blocked, a hit's Flinch, dynamic-param changes) set the character's
//!     ReplicatedNetMotion (COND_SkipOwner: simulated proxies get it as a property) and call ClientSetNetMotion on the
//!     owner (a reliable client RPC that writes ReplicatedNetMotion there and runs its OnRep).
//!   - On the owning client, an authoritative motion equal to the predicted one only confirms it; older confirmations
//!     the client already predicted past are consumed from UnconfirmedMotionsBacklog.
//! The motion objects themselves are created by `Pawn::begin_net_motion` (the HandleNetMotionUpdate tail).
//! Instead of calling a link, the rules queue what they send in `outbox` (the machine drains it into its transport).

use std::collections::VecDeque;

use serde::{Deserialize, Serialize};

use crate::consts::*;
use crate::enums::{self, combat};
use crate::msg::Msg;
use crate::netmotion::{compare, FNetMotion};
use crate::pawn::{is_confirmed, is_initiated_locally, Pawn};
use crate::stat;
use crate::ue::{clampf, cvtss2si, f32r};

/// A message the rules queued: a server RPC (client -> server) or a client RPC to the pawn's owning connection
#[derive(Clone, Debug, PartialEq)]
pub enum Out {
    Server(Msg),
    Owner(Msg),
}

/// AMordhauCharacter +0x10a8 FNetBlock {BlockedReason, Flags, BlockedMove, Surface, BlockingActor, Version}
/// (types/FNetBlock.h). Wire form: the 4 bytes + Version; BlockingActor travels as a fighter / weapon name.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetBlock {
    pub reason: u8,
    pub flags: u8,
    pub mv: u8,
    pub surface: u8,
    pub actor: String,
    pub version: u8,
}

/// The FBlockResult fields AssignNetBlock packs (missing = false / 0)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BlockResult {
    pub reason: u8,
    pub surface: u8,
    pub stun: bool,
    pub disarm: bool,
    pub ranged: bool,
    pub cancel: bool,
    pub party: bool,
    pub self_event: bool,
    pub clash_on_parry: bool,
}

impl BlockResult {
    /// Flags = bIsStun | bIsDisarm << 1 | bIsRanged << 2 | bIsCancel << 3 | bPartyFlag << 4 |
    /// bRequiresSelfBlockEvent << 5 | bClashOnParry << 6 (AMordhauCharacter::AssignNetBlock rva=0x1530ec0)
    pub fn flags(&self) -> u8 {
        let bits = [self.stun, self.disarm, self.ranged, self.cancel, self.party, self.self_event, self.clash_on_parry];
        bits.iter().enumerate().fold(0u8, |f, (i, b)| if *b { f | (1 << i) } else { f })
    }

    pub fn from_flags(reason: u8, surface: u8, f: u8) -> Self {
        let b = |i: u8| (f >> i) & 1 != 0;
        BlockResult { reason, surface, stun: b(0), disarm: b(1), ranged: b(2), cancel: b(3), party: b(4), self_event: b(5), clash_on_parry: b(6) }
    }
}

/// every FBlockResult OnRep_NetBlock decoded on this machine (the cosmetic adapters' input)
#[derive(Clone, Debug, PartialEq)]
pub struct AppliedBlock {
    pub result: BlockResult,
    pub mv: u8,
    pub actor: String,
    pub version: u8,
    pub t: f64,
}

#[derive(Clone, Debug, Default)]
pub struct NetMotionRep {
    pub who: String,
    /// AMordhauCharacter +0x10c9 ReplicatedNetMotion
    pub replicated: FNetMotion,
    /// UMotionSystemComponent +0xe2 LocallyPredictedNetMotion
    pub locally_predicted: FNetMotion,
    /// +0xe0 LastAcceptedClientMotionID
    pub last_accepted_client_motion_id: u8,
    /// +0xe1 LastAuthoritativeMotionID
    pub last_authoritative_motion_id: u8,
    /// +0xd0 UnconfirmedMotionsBacklog (newest first)
    pub backlog: VecDeque<FNetMotion>,
    /// AAdvancedCharacter +0x9d2 ReplicatedHealth (not set by the constructor: 0)
    pub replicated_health: u8,
    /// AMordhauCharacter +0xe68 ReplicatedStamina
    pub replicated_stamina: u8,
    // AAdvancedCharacter state the client hit suggestion reads (a game adapter sets these):
    /// +0x505 bIsRagdollFalling
    pub ragdoll_falling: bool,
    /// +0x844 RagdollFallingGetUpStartTime
    pub ragdoll_get_up_start_time: f64,
    /// +0x840 RagdollFallingGetUpDuration
    pub ragdoll_get_up_duration: f64,
    /// +0x7f0 bCanReceiveClientsideHits: the AAdvancedCharacter constructor does not write it (0); only
    /// Mordhau/Content/Mordhau/Blueprints/GameModes/Horde/BP_HordeEnemy sets it
    pub can_receive_clientside_hits: bool,
    /// AAdvancedCharacter +0x839 ReplicatedCharacterFlags
    pub replicated_character_flags: u8,
    /// AAdvancedCharacter +0x8ac ReplicatedLookUpValue
    pub replicated_look_up_value: u8,
    /// LookUpValueSmoothingTarget (what OnRep_ReplicatedLookUpValue aims at)
    pub look_up_smoothing_target: f64,
    /// ServerSuggestHitDetection calls the server ignored (tests)
    pub rejected_suggestions: u32,
    /// AMordhauCharacter +0x10a8 NetBlock (the constructor rva=0x1524d90 zeroes it)
    pub net_block: NetBlock,
    /// +0x10b8 LastNetBlockVersion (the constructor sets -1)
    pub last_net_block_version: i32,
    pub net_blocks_applied: Vec<AppliedBlock>,
    /// AMordhauCharacter +0xd5d ReplicatedKnockback (COND_None, push-model)
    pub replicated_knockback: u8,
    /// +0xe90 LastKnockback
    pub last_knockback: f64,
    /// UMordhauMovementComponent::IsInKnockback (+0xd4c > 0): movement state, set by a movement adapter
    pub in_knockback: bool,
    /// Amount vectors Knockback accepted on this machine (the movement adapter's input)
    pub knockbacks: Vec<[f32; 3]>,
    /// AMordhauCharacter +0xd5c ReplicatedDodge (COND_SimulatedOnly)
    pub replicated_dodge: u8,
    /// (time, world yaw) of every OnDodged on this machine (the cosmetic adapter's input)
    pub dodges: Vec<(f64, f64)>,
    /// what the rules sent, in order (drained by NetServer / NetClient)
    pub outbox: Vec<Out>,
    /// the owning connection's endpoint (its player state), None = no owner
    pub owner: Option<u32>,
}

impl NetMotionRep {
    pub fn new(who: &str) -> Self {
        NetMotionRep { who: who.to_string(), last_net_block_version: -1, ..Default::default() }
    }

    fn csnm(&self, nm: &FNetMotion, t: f64) -> Msg {
        Msg::ClientSetNetMotion { who: self.who.clone(), nm: nm.to_bytes(), t }
    }

    /// from UMotionSystemComponent::AssignNetMotion rva=0x14b34c0 (Id already set by the caller, MotionSystem._assign:
    /// NewNetMotion.Id = NetMotion.Id + 1):
    ///   bIsDead -> nothing
    ///   Role == 3: ReplicatedNetMotion = new; ClientSetNetMotion(new, World RealTimeSeconds) to the owner; vcall +0x5f0
    ///     (UNCONFIRMED: ForceNetUpdate; replication here runs every server frame anyway)
    ///   else: ServerAssignNetMotion(new, LastAuthoritativeMotionID); LocallyPredictedNetMotion = new
    ///   HandleNetMotionUpdate(bIsLocalSimulation = Role != 3, bSkipDeltaTimeForward)
    pub fn assign_net_motion<P: Pawn + ?Sized>(&mut self, p: &mut P, nm: FNetMotion) {
        if p.is_dead() {
            return;
        }
        if p.role() == enums::ROLE_AUTHORITY {
            self.replicated = nm;
            let m = self.csnm(&nm, p.now());
            self.outbox.push(Out::Owner(m));
        } else {
            self.outbox.push(Out::Server(Msg::ServerAssignNetMotion { who: self.who.clone(), nm: nm.to_bytes(), last: self.last_authoritative_motion_id }));
            self.locally_predicted = nm;
        }
        let local = p.role() != enums::ROLE_AUTHORITY;
        self.handle_net_motion_update(p, local);
    }

    /// from UMotionSystemComponent::OnServerAssignNetMotion rva=0x14cf410 (AMordhauCharacter::
    /// ServerAssignNetMotion_Implementation rva=0x1567ab0 is `mov rcx, [rcx+0x688]; test; jne` to it):
    ///   accept when NewNetMotion.Id == NetMotion.Id + 1 and (LastAuthObserved == NetMotion.Id or
    ///   LastAcceptedClientMotionID + 1 == NetMotion.Id + 1); then LastAcceptedClientMotionID = that Id and
    ///   AssignNetMotion(NewNetMotion, false). Anything else is dropped silently (the server's own newer motion wins).
    pub fn on_server_assign_net_motion<P: Pawn + ?Sized>(&mut self, p: &mut P, nm: FNetMotion, last_auth_observed: u8) -> bool {
        let cur = p.net().id;
        let nxt = cur.wrapping_add(1);
        if nm.id != nxt {
            return false;
        }
        if last_auth_observed != cur && self.last_accepted_client_motion_id.wrapping_add(1) != nxt {
            return false;
        }
        self.last_accepted_client_motion_id = nxt;
        let mut c = nm;
        c.id = nxt; // AssignNetMotion: Id = NetMotion.Id + 1 again (the same value)
        self.assign_net_motion(p, c);
        true
    }

    /// from UMotionSystemComponent::OnClientSetNetMotion rva=0x14ca1c0 (AMordhauCharacter::
    /// ClientSetNetMotion_Implementation rva=0x1535cb0 calls it): only in net mode 3 (a client): ReplicatedNetMotion =
    /// NewMotion, then OnRep_ReplicatedNetMotion. ServerStartTime is not read.
    pub fn on_client_set_net_motion<P: Pawn + ?Sized>(&mut self, p: &mut P, nm: FNetMotion, _server_start_time: f64) {
        if p.slot().ctx.net_mode != enums::NET_MODE_CLIENT as i64 {
            return;
        }
        self.replicated = nm;
        self.on_rep_replicated_net_motion(p);
    }

    /// from UMotionSystemComponent::OnRep_ReplicatedNetMotion rva=0x14ce7d0 (AMordhauCharacter::
    /// OnRep_ReplicatedNetMotion rva=0x155ad00 tail-jumps to it):
    ///   controller role 2 (the owning client) and LastAuthoritativeMotionID == ReplicatedNetMotion.Id and the current
    ///   motion is not yet confirmed -> only LastAuthoritativeMotionID = Id
    ///   else LastAuthoritativeMotionID = Id; HandleNetMotionUpdate(false, false)
    pub fn on_rep_replicated_net_motion<P: Pawn + ?Sized>(&mut self, p: &mut P) {
        if p.role() == enums::ROLE_AUTONOMOUS_PROXY && self.last_authoritative_motion_id == self.replicated.id && p.has_motion() && !is_confirmed(p) {
            self.last_authoritative_motion_id = self.replicated.id;
            return;
        }
        self.last_authoritative_motion_id = self.replicated.id;
        self.handle_net_motion_update(p, false);
    }

    /// from UMotionSystemComponent::HandleNetMotionUpdate rva=0x14c01b0:
    ///   bIsDead -> nothing. Candidate = LocallyPredictedNetMotion (local) or ReplicatedNetMotion; d = compare(Candidate,
    ///   NetMotion).
    ///   local: the current motion unconfirmed -> NetMotion goes to the FRONT of UnconfirmedMotionsBacklog.
    ///   authoritative: pop the backlog front while it is not empty; a Candidate equal to it (d 0 or 1) was already
    ///     predicted past -> return. Then d < 2 and the current motion unconfirmed and bInitiatedLocally -> it is
    ///     confirmed (ConfirmedByAuthorityTime = World TimeSeconds).
    ///   d 0 -> return; d 1 -> NetMotion.DynamicParam = Candidate's, Motion->DynamicParamChanged(old, new);
    ///   d 2 -> NetMotion = Candidate, a new motion object for it (local: bInitiatedLocally; authoritative:
    ///     bWasConfirmedByAuthority, ConfirmedByAuthorityTime, and on the owning client ExpectedDelay = ping x
    ///     WorldSettings +0x2e8), ChangeMotion_Internal, DynamicParamChanged(0, DynamicParam) when not 0.
    /// Not ported: AMordhauEquipmentRemapper::RemapMotion (no remapper in a duel) and the AMordhauPlayerController
    /// ServerHasPassedCheck on an attack that failed to create.
    /// bSkipDeltaTimeForward goes to ChangeMotion_Internal -> UMordhauMotion::Initialize(bAppendAppDeltaTime)
    /// rva=0x165d4c0. Every caller in the game module passes false (a capstone scan of all game functions: each call
    /// of AssignNetMotion 0x1414b34c0, ChangeMotion 0x1414b4e70 and HandleNetMotionUpdate 0x1414c01b0 sets r8d to 0
    /// or forwards its caller's), so the step never runs in game code and is not ported.
    pub fn handle_net_motion_update<P: Pawn + ?Sized>(&mut self, p: &mut P, local: bool) {
        if p.is_dead() {
            return;
        }
        let cand = if local { self.locally_predicted } else { self.replicated };
        let net = p.net();
        let d = compare(&cand, &net);
        if local {
            if p.has_motion() && !is_confirmed(p) {
                self.backlog.push_front(net);
            }
        } else {
            while let Some(front) = self.backlog.pop_front() {
                if compare(&cand, &front) != 2 {
                    return;
                }
            }
            if d < 2 && p.has_motion() && !is_confirmed(p) && is_initiated_locally(p) {
                let now = p.now();
                let s = p.slot_mut();
                s.confirmed = true;
                s.confirmed_by_authority_time = now;
            }
        }
        if d == 0 {
            return;
        }
        if d == 1 {
            let mut n = net;
            let old = n.dynamic_param;
            n.dynamic_param = cand.dynamic_param;
            p.set_net(n);
            if p.has_motion() {
                p.on_dynamic_param_changed(old, n.dynamic_param);
            }
            p.trace_event(&format!("dyn {}", n.dynamic_param));
            return;
        }
        p.set_net(cand);
        let mut delay = 0.0f64;
        if !local {
            let now = p.now();
            p.slot_mut().confirmed_by_authority_time = now;
            // GetControllerRoleIncludingVehicle == 2 (the owning client): ExpectedDelay = GetPing(bUseMedian) x
            // WorldSettings TimeDilation (+0x2e8; AWorldSettings::SetTimeDilation rva=0x357b850 writes it there)
            if p.role() == enums::ROLE_AUTONOMOUS_PROXY {
                let c = p.slot().ctx;
                delay = f32r(c.ping * c.time_dilation);
            }
        }
        p.slot_mut().pending = Some((local, delay));
        p.begin_net_motion();
        p.slot_mut().pending = None;
    }

    /// from UMotionSystemComponent::AssignNetMotionDynamicParam rva=0x14b35f0: only alive and Role == 3:
    /// ReplicatedNetMotion.DynamicParam = new; ClientSetNetMotion(ReplicatedNetMotion, RealTimeSeconds) to the owner;
    /// OnRep_ReplicatedNetMotion (runs HandleNetMotionUpdate here on the server); vcall +0x5f0. Clients never set it.
    pub fn assign_net_motion_dynamic_param<P: Pawn + ?Sized>(&mut self, p: &mut P, v: u8) {
        if p.is_dead() || p.role() != enums::ROLE_AUTHORITY {
            return;
        }
        self.replicated.dynamic_param = v;
        let m = self.csnm(&self.replicated.clone(), p.now());
        self.outbox.push(Out::Owner(m));
        self.on_rep_replicated_net_motion(p);
    }

    /// The RPC thunk the parry calls, `AMordhauCharacter::ServerDropParry` 0x1416a5c10, from a client: a reliable
    /// server RPC with the caller's NetMotion Id; the server runs ServerDropParry_Implementation (the combat crate's
    /// MotionSystem.server_drop_parry_implementation).
    pub fn send_server_drop_parry(&mut self, motion_id: u8) {
        self.outbox.push(Out::Server(Msg::ServerDropParry { who: self.who.clone(), id: motion_id }));
    }

    /// UHealthStatComponent::SetStatValue_Internal rva=0x14d5cf0 with bReplicate: WriteReplicatedStat(&ReplicatedHealth)
    pub fn write_replicated_health<P: Pawn + ?Sized>(&mut self, p: &P) {
        let (mn, mx) = p.health_range();
        self.replicated_health = stat::write_replicated_stat_q(p.health(), mn, mx, self.replicated_health, p.q());
    }

    /// UStaminaStatComponent::SetStatValue_Internal rva=0x1515c80 with bReplicate: WriteReplicatedStat(&ReplicatedStamina)
    pub fn write_replicated_stamina<P: Pawn + ?Sized>(&mut self, p: &P) {
        let (mn, mx) = p.stamina_range();
        self.replicated_stamina = stat::write_replicated_stat_q(p.stamina(), mn, mx, self.replicated_stamina, p.q());
    }

    /// AMordhauCharacter::OnRep_ReplicatedHealth rva=0x155abd0 -> AAdvancedCharacter::OnRep_ReplicatedHealth
    /// rva=0x1491cc0 -> ReadReplicatedStat(ReplicatedHealth); a changed value goes through SetStatValue_Internal (OnDied
    /// at > 0 -> < 1). (The rest of the Mordhau OnRep is local stats: LocallyTrackedDamageReceived,
    /// bHasEverReachedOneHealth.)
    pub fn on_rep_replicated_health<P: Pawn + ?Sized>(&mut self, p: &mut P) {
        let (mn, mx) = p.health_range();
        let v = stat::read_replicated_stat_q(self.replicated_health, mn, mx, p.q());
        if v != p.health() {
            p.set_health_internal(v);
        }
    }

    /// AMordhauCharacter::OnRep_ReplicatedStamina rva=0x155aef0 -> ReadReplicatedStat(ReplicatedStamina)
    /// Stamina regeneration is NOT replicated and runs on every machine (resolved from the exe, was UNCONFIRMED):
    /// AMordhauCharacter::LODTick rva=0x154c390 calls the stamina component's TickStat (vcall +0x410 on +0x6a8 at
    /// 0x14154ce23) after the Role == 3 block (`cmp byte [rbx+0xf0], 3` at 0x14154cd33) has rejoined at 0x14154cd78, so
    /// with no role test; UStatComponent::TickStat rva=0x1519ab0 writes each step with SetStatValue_Internal(v, false)
    /// (no ReplicatedStamina write), and UStaminaStatComponent::OffsetStamina rva=0x1507c20 changes the stat only on
    /// Role 3 (`(pAVar5->Role).Value == 3`). StopRegeneration calls made by authority-only rules (hit processing,
    /// ReceiveBlock) therefore never reach a client, which can regenerate earlier until the next ReplicatedStamina
    /// write resets it: a client's stamina may lead the server's between writes, as in the shipped game.
    pub fn on_rep_replicated_stamina<P: Pawn + ?Sized>(&mut self, p: &mut P) {
        let (mn, mx) = p.stamina_range();
        let v = stat::read_replicated_stat_q(self.replicated_stamina, mn, mx, p.q());
        if v != p.stamina() {
            p.set_stamina_internal(v);
        }
    }

    // ---- client hit suggestion ---------------------------------------------------------------------------------
    /// AAdvancedCharacter::IsRagdollFallingOrGettingUp rva=0x1487ca0: bIsRagdollFalling, or now < GetUpStartTime +
    /// GetUpDuration
    pub fn is_ragdoll_falling_or_getting_up(&self, now: f64) -> bool {
        if self.ragdoll_falling {
            return true;
        }
        !(self.ragdoll_get_up_start_time + self.ragdoll_get_up_duration <= now)
    }

    /// The client half, from UAttackMotion::OnLateTick_Implementation rva=0x1631f00 (decomp UAttackMotion.cpp
    /// 5455-5491): for each cosmetic hit of this pawn's weapon on an AAdvancedCharacter's mesh, while the owner
    /// IsLocallyControlledIncludingVehicle (vcall +0x9c8) and its Role != 3: when the victim's Role != 3, it is alive
    /// and IsRagdollFallingOrGettingUp (vcall +0x960 resolves to AAdvancedCharacter::IsRagdollFallingOrGettingUp in
    /// both character vtables), the client sends ServerSuggestHitDetection(victim, location, bone index).
    /// Returns whether the RPC was sent.
    pub fn cosmetic_hit(&mut self, role: u8, victim_rep: &NetMotionRep, victim: &crate::world::PawnInfo, bone: &str) -> bool {
        if role != enums::ROLE_AUTONOMOUS_PROXY {
            return false;
        }
        if victim.role == enums::ROLE_AUTHORITY || victim.dead || !victim_rep.is_ragdoll_falling_or_getting_up(victim.now) {
            return false;
        }
        self.outbox.push(Out::Server(Msg::ServerSuggestHitDetection { who: self.who.clone(), other: victim_rep.who.clone(), bone: bone.to_string() }));
        true
    }

    /// the victim half of the ServerSuggestHitDetection_Implementation gate: (IsRagdollFallingOrGettingUp or
    /// bCanReceiveClientsideHits); see server.rs for the rest of the rule
    pub fn accepts_suggested_hit(&self, now: f64) -> bool {
        self.is_ragdoll_falling_or_getting_up(now) || self.can_receive_clientside_hits
    }

    // ---- ReplicatedCharacterFlags / ReplicatedLookUpValue ------------------------------------------------------
    /// Authority, from AAdvancedCharacter::LODTick rva=0x14887b0 (Role 3: bit 0 = the movement component's IsFalling,
    /// vcall +0x560 -> here Pawn::airborne) and AAdvancedCharacter::SetIsRagdollFalling rva=0x14a0980 (bit 2).
    /// Bit 1 (burning) and bit 3 (flipped by OnJumped_Implementation rva=0x148d580) have no state in this world.
    pub fn write_character_flags<P: Pawn + ?Sized>(&mut self, p: &P) {
        let mut f = self.replicated_character_flags & !0x5;
        if p.airborne() {
            f |= 1;
        }
        if self.ragdoll_falling {
            f |= 4;
        }
        self.replicated_character_flags = f;
    }

    /// AAdvancedCharacter::OnRep_ReplicatedCharacterFlags rva=0x1491c20: nothing when bIsDead; bit 2 -> vcall +0xa58
    /// (SetIsRagdollFalling). Bit 0 is read where it is needed: AAdvancedCharacter::IsAirborne rva=0x14873e0 returns
    /// ReplicatedCharacterFlags & 1 on every machine.
    pub fn on_rep_replicated_character_flags<P: Pawn + ?Sized>(&mut self, p: &mut P) {
        p.set_airborne(self.replicated_character_flags & 1 != 0);
        if p.is_dead() {
            return;
        }
        self.ragdoll_falling = self.replicated_character_flags & 4 != 0;
    }

    /// AAdvancedCharacter::PreReplication rva=0x1499d60 (authority, before every replication pass):
    ///   span = LookUpLimit + LookDownLimit; f = |span| > 1e-8 ? (LookUpValue + LookDownLimit) / span :
    ///   (LookUpValue < LookUpLimit ? 0 : 1); ReplicatedLookUpValue = (uint8)(int)(clamp01(f) x 255)
    pub fn write_look_up<P: Pawn + ?Sized>(&mut self, p: &P) {
        let (up, down) = p.look_limits();
        let span = f32r(up + down);
        let mut f = LOOK_UNIT_MAX as f64;
        if span.abs() > LOOK_SMALL_NUMBER as f64 {
            f = f32r(f32r(p.look_up_value() + down) / span);
        } else if p.look_up_value() < up {
            f = 0.0;
        }
        f = clampf(f, 0.0, LOOK_UNIT_MAX as f64);
        self.replicated_look_up_value = (f32r(f * LOOK_UP_TO_BYTE as f64) as i64 & 0xff) as u8;
    }

    /// AAdvancedCharacter::OnRep_ReplicatedLookUpValue rva=0x1491cf0 (COND_SimulatedOnly): SmoothingFrom =
    /// LookUpValue; SmoothingTarget = (LookUpLimit + LookDownLimit) x clamp01(byte / 255) - LookDownLimit; the
    /// smoothing window uses the movement component's NetworkSimulatedSmoothRotationTime. UNCONFIRMED / not ported:
    /// the per-tick smoothing from SmoothingFrom to SmoothingTarget (AAdvancedCharacter tick), so the target is taken
    /// at once.
    pub fn on_rep_replicated_look_up_value<P: Pawn + ?Sized>(&mut self, p: &mut P) {
        let (up, down) = p.look_limits();
        let f = clampf(f32r(self.replicated_look_up_value as f64 * LOOK_UP_BYTE_TO_UNIT as f64), 0.0, LOOK_UNIT_MAX as f64);
        self.look_up_smoothing_target = f32r(f32r(f32r(up - -down) * f) + -down);
        p.set_look_up_value(self.look_up_smoothing_target);
    }

    // ---- NetBlock ----------------------------------------------------------------------------------------------
    /// from AMordhauCharacter::AssignNetBlock rva=0x1530ec0: Flags from the FBlockResult bools (BlockResult::flags);
    /// Reason, Surface, BlockedMove copied; BlockingActor = Weapon; ++Version (uint8); OnRep_NetBlock() here; vcall
    /// +0x5f0 ForceNetUpdate (replication runs every server frame here).
    pub fn assign_net_block<P: Pawn + ?Sized>(&mut self, p: &P, result: &BlockResult, blocked_move: u8, weapon: &str) {
        self.net_block = NetBlock {
            reason: result.reason,
            flags: result.flags(),
            mv: blocked_move,
            surface: result.surface,
            actor: weapon.to_string(),
            version: self.net_block.version.wrapping_add(1),
        };
        self.on_rep_net_block(p);
    }

    /// from AMordhauCharacter::OnRep_NetBlock rva=0x155a4e0 (the net half):
    ///   World TimeSeconds <= 7 -> return (nothing in a level's first 7 s, LastNetBlockVersion not updated)
    ///   LastNetBlockVersion != -1 and == Version -> return (already applied)
    ///   LastNetBlockVersion = Version; FBlockResult from Flags (the AssignNetBlock bit order)
    /// The rest is cosmetic and lives in the adapters (net_blocks_applied): UMordhauStats Blocks / Chambers counters,
    /// the anim instance BlockDirection and ParryPush additive, the BlockShakeEffect camera shake, and the weapon's
    /// OnBlocked / OnWasBlocked effects.
    pub fn on_rep_net_block<P: Pawn + ?Sized>(&mut self, p: &P) {
        if p.now() <= NET_BLOCK_MIN_WORLD_TIME as f64 {
            return;
        }
        let v = self.net_block.version as i32;
        if self.last_net_block_version != -1 && self.last_net_block_version == v {
            return;
        }
        self.last_net_block_version = v;
        let nb = &self.net_block;
        self.net_blocks_applied.push(AppliedBlock {
            result: BlockResult::from_flags(nb.reason, nb.surface, nb.flags),
            mv: nb.mv,
            actor: nb.actor.clone(),
            version: nb.version,
            t: p.now(),
        });
    }

    /// Hook from the parry's ReceiveBlock (UParryMotion::ReceiveBlock rva=0x166bf90, the branch taken when the
    /// parrier's Stamina byte != 0 or the attack is Ranged): AssignNetBlock({Reason Parry, bIsRanged = Move ==
    /// Ranged}, AttackMove, null), then, for a non-ranged block outside a shield wall,
    /// ApplyBackwardsKnockbackIfNotInKnockback(KnockbackParry).
    pub fn on_receive_block<P: Pawn + ?Sized>(&mut self, p: &P, attacker_move: u8, shield_wall: bool) {
        let ranged = attacker_move == combat::MOVE_RANGED;
        self.assign_net_block(p, &BlockResult { reason: combat::BLOCKED_PARRY, ranged, ..Default::default() }, attacker_move, "");
        if !ranged && !shield_wall {
            self.apply_backwards_knockback_if_not_in_knockback(p, p.knockback_parry());
        }
    }

    // ---- ReplicatedKnockback -----------------------------------------------------------------------------------
    /// AMordhauCharacter::ApplyBackwardsKnockbackIfNotInKnockback rva=0x1530600: Amount > 0 and not IsInKnockback ->
    /// Knockback(-Forward x Amount) (vcall +0x998 = AMordhauCharacter::Knockback). Forward = the root component's
    /// forward, FVector::ForwardVector without one.
    pub fn apply_backwards_knockback_if_not_in_knockback<P: Pawn + ?Sized>(&mut self, p: &P, amount: f64) {
        if !(0.0 < amount) || self.in_knockback {
            return;
        }
        let mut fwd = p.forward().unwrap_or([1.0, 0.0, 0.0]);
        let len = (fwd[0] * fwd[0] + fwd[1] * fwd[1] + fwd[2] * fwd[2]).sqrt();
        if len > 0.0 {
            fwd = [fwd[0] / len, fwd[1] / len, fwd[2] / len];
        }
        // -Forward x Amount as a Vector3 (f32 components)
        let a = amount as f32;
        self.knockback(p, [-fwd[0] * a, -fwd[1] * a, -fwd[2] * a]);
    }

    /// AMordhauCharacter::Knockback rva=0x154c290: Role != 3 or |Amount| <= 0.1 -> false; (a current vehicle -> false:
    /// no vehicles here); LastKnockback = TimeSeconds; UMordhauMovementComponent::Knockback(Amount) (not ported:
    /// knockbacks); ++ReplicatedKnockback; vcall +0x5f0 ForceNetUpdate; true.
    pub fn knockback<P: Pawn + ?Sized>(&mut self, p: &P, amount: [f32; 3]) -> bool {
        let len = (amount[0] * amount[0] + amount[1] * amount[1] + amount[2] * amount[2]).sqrt();
        if p.role() != enums::ROLE_AUTHORITY || len as f64 <= KNOCKBACK_MIN_SIZE as f64 {
            return false;
        }
        self.last_knockback = p.now();
        self.knockbacks.push(amount);
        self.replicated_knockback = self.replicated_knockback.wrapping_add(1);
        true
    }

    /// AMordhauCharacter::OnRep_ReplicatedKnockback rva=0x155ac90: LastKnockback = TimeSeconds; the movement
    /// component's knockback friction / velocity reset (UMordhauMovementComponent fields) is engine movement: not
    /// ported.
    pub fn on_rep_replicated_knockback<P: Pawn + ?Sized>(&mut self, p: &P) {
        self.last_knockback = p.now();
    }

    // ---- ReplicatedDodge ---------------------------------------------------------------------------------------
    /// The owning client's request, from UMordhauMovementComponent::LODTick rva=0x14c4d60 (decomp lines 1478-1503;
    /// the dodge-start conditions before it are movement, not ported): yaw = Atan2(Dir.Y, Dir.X) x 57.295776; v = yaw <
    /// 0 ? yaw + 360 : yaw, clamped to [0, 360]; PackedWorldYaw = clamp(cvtss2si(v x k + v x k + 0.5) >> 1, 0, 255)
    /// with k = 255/360; ServerRequestDodge(PackedWorldYaw); Role != 3 -> OnDodged(yaw) at once (vcall +0xb38).
    /// dir: the dodge direction in UE axes (X forward, Y right). Returns the packed byte.
    pub fn request_dodge<P: Pawn + ?Sized>(&mut self, p: &mut P, dir: [f32; 2]) -> u8 {
        let yaw = f32r(f32r((dir[1] as f64).atan2(dir[0] as f64)) * DODGE_RAD_TO_DEG as f64);
        let turn = DODGE_UNWIND_TURN as f64;
        let mut v = yaw;
        if v < 0.0 {
            v = f32r(v + turn);
            if v < 0.0 {
                v = 0.0;
            }
        }
        if v >= turn {
            v = turn;
        }
        let k = f32r(v * DODGE_DEG_TO_BYTE as f64);
        let packed = (cvtss2si(f32r(f32r(k + k) + DODGE_ROUND_HALF as f64)) >> 1).clamp(0, 0xff) as u8;
        if p.role() == enums::ROLE_AUTHORITY {
            self.server_request_dodge(p, packed);
        } else {
            self.outbox.push(Out::Server(Msg::ServerRequestDodge { who: self.who.clone(), yaw: packed }));
            self.on_dodged(p, yaw);
        }
        packed
    }

    /// The unwind loops both dodge functions inline: while > 180: -= 360; while < -180: += 360
    pub fn unwind_dodge(mut a: f64) -> f64 {
        while (DODGE_UNWIND_MAX as f64) < a {
            a = f32r(a + DODGE_UNWIND_NEG_TURN as f64);
        }
        while a < DODGE_UNWIND_MIN as f64 {
            a = f32r(a + DODGE_UNWIND_TURN as f64);
        }
        a
    }

    /// AMordhauCharacter::ServerRequestDodge_Implementation rva=0x1567ca0 (_Validate rva=0x7bf3e0 returns true):
    /// ReplicatedDodge = PackedWorldYaw, or PackedWorldYaw + 1 when it already holds that value (so a repeat dodge in
    /// the same direction still changes the property and the simulated proxies' OnRep runs);
    /// OnDodged(unwind(byte x 360/255)).
    pub fn server_request_dodge<P: Pawn + ?Sized>(&mut self, p: &mut P, packed: u8) {
        let mut b = packed;
        if self.replicated_dodge == b {
            b = b.wrapping_add(1);
        }
        self.replicated_dodge = b;
        self.on_dodged(p, Self::unwind_dodge(f32r(b as f64 * DODGE_BYTE_TO_DEG as f64)));
    }

    /// AMordhauCharacter::OnRep_ReplicatedDodge rva=0x155ab10: OnDodged(unwind(ReplicatedDodge x 360/255))
    pub fn on_rep_replicated_dodge<P: Pawn + ?Sized>(&mut self, p: &mut P) {
        let y = Self::unwind_dodge(f32r(self.replicated_dodge as f64 * DODGE_BYTE_TO_DEG as f64));
        self.on_dodged(p, y);
    }

    /// AMordhauCharacter::OnDodged rva=0x1551be0, the state half: StaminaStatComponent->OffsetStamina(-DodgeStaminaCost)
    /// (authority only) and StopRegeneration(0). The rest (dodge sound, camera shake and CameraRotation1P yaw, particle
    /// effect at the floor) is cosmetic: `dodges` for an adapter.
    pub fn on_dodged<P: Pawn + ?Sized>(&mut self, p: &mut P, world_yaw: f64) {
        let cost = p.dodge_stamina_cost();
        if p.offset_stamina_raw(-cost) {
            self.write_replicated_stamina(p);
        }
        p.stop_stamina_regen(0.0);
        self.dodges.push((p.now(), world_yaw));
    }
}

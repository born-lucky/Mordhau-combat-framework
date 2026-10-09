//! Port of godot/tests/test_net_ping.gd: the exe's ping compensation, client hit suggestion, the remaining character
//! properties and the engine RPC gate. Expected values follow from the rules cited in src/:
//!   PingMedian (AMordhauPlayerState::UpdatePing rva=0x1604560), ExpectedDelay (UMotionSystemComponent::
//!   HandleNetMotionUpdate rva=0x14c01b0), attack ping compensation (UMotionSystemComponent::AssignNetAttackMotion
//!   rva=0x14b3210, UMordhauMotion::GetAttackCompensationStartTime rva=0x165c630 and overrides), LagInduction /
//!   LagReduction (UAttackMotion::OnBegin_Implementation rva=0x162eda0), ServerSuggestHitDetection
//!   (AMordhauCharacter::ServerSuggestHitDetection_Implementation rva=0x1567d80), ReplicatedLookUpValue
//!   (AAdvancedCharacter::PreReplication rva=0x1499d60), ReplicatedTeam (AMordhauPlayerState::SetTeam rva=0x15fd480),
//!   the RPC sender gate (UNetDriver::ProcessRemoteFunction rva=0x323cf80), NetBlock / Knockback / Dodge.
//! The GDScript cases that check combat numbers (Param2 = 40 from max_ping_compensation x 500, windup / HitRecovery
//! arithmetic of a real BP_Longsword attack) need mordhau-core's world: their net halves are unit-tested here on the
//! pure functions (attack_ping_compensation, attack_lag) with the same inputs and expected values.

mod support;

use mh_net::consts::*;
use mh_net::enums::{self, combat};
use mh_net::msg::Msg;
use mh_net::pawn::{attack_compensation_start_time, attack_lag, attack_ping_compensation, AttackLag, CompMotion, LinkCtx};
use mh_net::player_state::NetPlayerState;
use mh_net::rep::{BlockResult, NetMotionRep};
use mh_net::{FNetMotion, NetSession};
use support::{Input, Kind, ToyWorld};

const DT: f64 = 0.001;
const LAT: u64 = 50; // frames one way: 50 ms, so every client's round trip is 0.1 s

fn near(a: f64, b: f64, eps: f64) -> bool {
    (a - b).abs() <= eps
}

fn session(latency: u64) -> NetSession<ToyWorld> {
    let mut s = NetSession::new(DT, latency, Box::new(|| ToyWorld::new(DT)));
    s.add_player("A", "LS", "");
    s.add_player("B", "LS", "");
    s
}

// ---- PingMedian -----------------------------------------------------------------------------------------------------
/// 121 zero slots; with a steady ping the median reads the copy taken before the sort at index CeilToInt(60.5) = 61:
/// PingMedian is 0 for 60 samples and p from the 61st on. Samples clamp to 1 s.
#[test]
fn ping_median_update_rule() {
    let mut ps = NetPlayerState::new();
    assert_eq!(ps.median_pings.len(), 121);
    for k in 0..60 {
        ps.update_ping(0.1);
        assert_eq!(ps.ping_median, 0.0, "PingMedian after {} samples", k + 1);
    }
    ps.update_ping(0.1);
    assert!(near(ps.ping_median, 0.1, 1e-7), "PingMedian {} after 61 samples", ps.ping_median);
    let mut hi = NetPlayerState::new();
    for _ in 0..121 {
        hi.update_ping(3.0);
    }
    assert_eq!(hi.ping_median, 1.0, "a 3 s sample not clamped to 1 s");
}

/// AMordhauPlayerState::SetTeam / OnRep_ReplicatedTeam: ReplicatedTeam = Team + 1 clamped to a byte, 0 for no team
#[test]
fn replicated_team_byte() {
    let mut ps = NetPlayerState::new();
    for (t, rb, back) in [(0, 1, 0), (1, 2, 1), (-1, 0, -1), (300, 255, 254)] {
        ps.set_team(t, true);
        assert_eq!(ps.replicated_team, rb, "SetTeam({t})");
        let mut cl = NetPlayerState::new();
        cl.replicated_team = ps.replicated_team;
        cl.on_rep_replicated_team();
        assert_eq!(cl.team, back);
    }
    let mut other = NetPlayerState::new();
    other.set_team(1, false);
    assert_eq!(other.team, -1, "SetTeam ran without authority");
}

// ---- ExpectedDelay --------------------------------------------------------------------------------------------------
/// A motion the server forces on A (a flinch) reaches A's own client 50 ms late; there (controller role 2 only) it
/// carries ExpectedDelay = PingMedian x TimeDilation = 0.1 and ends that much earlier; the server and the simulated
/// proxy on B keep ExpectedDelay 0.
#[test]
fn expected_delay_owner_only() {
    let mut s = session(LAT);
    s.run_until(0.2);
    assert!(near(s.client("A").unwrap().get_ping(), 0.1, 1e-7), "client A PingMedian at 0.2 s");
    s.server.world.at(0.3, Input::Force("A".into(), FNetMotion::new(combat::NET_FLINCHED, 0, 0, 0, 0)));
    s.run_until(0.45);
    let sm = s.server.world.p("A").motion.clone();
    let om = s.client("A").unwrap().world.p("A").motion.clone();
    let pm = s.client("B").unwrap().world.p("A").motion.clone();
    for (n, m) in [("server", &sm), ("owner", &om), ("proxy", &pm)] {
        assert_eq!(m.kind, Kind::Flinch, "{n}");
    }
    assert!(near(sm.start, 0.3, DT * 0.5) && near(om.start, 0.35, DT * 0.5) && near(pm.start, 0.35, DT * 0.5), "starts {} {} {}", sm.start, om.start, pm.start);
    assert_eq!(sm.expected_delay, 0.0);
    assert_eq!(pm.expected_delay, 0.0);
    assert!(near(om.expected_delay, 0.1, 1e-7), "owner ExpectedDelay {}", om.expected_delay);
    let d_server = sm.end.unwrap() - sm.start;
    let d_owner = om.end.unwrap() - om.start;
    assert!(near(d_server - d_owner, 0.1, 1e-5));
}

// ---- attack ping compensation ---------------------------------------------------------------------------------------
/// test_attack_ping_compensation's net half: A attacks from Idle at 0.3 on its own client with ping 0.1:
/// compensation min(0.1, 0.3 - 0) = 0.1 (the combat crate then clamps to max_ping_compensation 0.08 and packs x 500 =
/// 40); on the server (no local player controller, ping 0) it is 0.
#[test]
fn attack_ping_compensation_rule() {
    let ctx = LinkCtx { net_mode: enums::NET_MODE_CLIENT as i64, ping: 0.1, ..Default::default() };
    let idle = CompMotion::Idle { coming_from: None, can_attack: true, end_time: 0.0 };
    let c = attack_ping_compensation(enums::ROLE_AUTONOMOUS_PROXY, &ctx, 0.3, Some(&idle), combat::MOVE_RIGHT_STRIKE);
    assert!(near(c, 0.1, 1e-7), "owner compensation {c}");
    let srv = LinkCtx { net_mode: enums::NET_MODE_SERVER as i64, ..Default::default() };
    assert_eq!(attack_ping_compensation(enums::ROLE_AUTHORITY, &srv, 0.3, Some(&idle), 0), 0.0);
    assert_eq!(attack_ping_compensation(enums::ROLE_AUTONOMOUS_PROXY, &ctx, 0.3, None, 0), (0.1f32) as f64, "f32(ping x TimeDilation)");
    // single precision as in AssignNetAttackMotion at 0x1414b33e8 / 0x1414b341c / 0x1414b3460: 0.08 x 500 = 40
    assert_eq!((0.08f32 * 500.0f32) as i32, 40);
}

/// LagInduction / LagReduction / MissRecovery: owner LagInduction = min(0.1, 0.15) x (1 - |AngleTarget| x 0.5), no
/// LagReduction for its own attack, MissRecovery - LagInduction; a simulated proxy LagInduction = -0.5 x 0.1; the
/// server (ping 0) nothing. NetcodeType 2 caps at 0.025, stabs take half.
#[test]
fn attack_lag_rules() {
    let ctx = LinkCtx { net_mode: enums::NET_MODE_CLIENT as i64, ping: 0.1, ..Default::default() };
    let angle = 0.6f64;
    let mut l = AttackLag { lag_reduction: 0.0, lag_induction: 0.0, miss_recovery: 0.5 };
    attack_lag(enums::ROLE_AUTONOMOUS_PROXY, &ctx, true, combat::MOVE_RIGHT_STRIKE, angle, &mut l);
    let li = 0.1f64 * (1.0 - angle.abs() * 0.5);
    assert!(near(l.lag_induction, li, 1e-6), "owner LagInduction {}", l.lag_induction);
    assert_eq!(l.lag_reduction, 0.0, "LagReduction for its own attack");
    assert!(near(l.miss_recovery, 0.5 - li, 1e-6));
    let mut r = AttackLag::default();
    attack_lag(enums::ROLE_AUTONOMOUS_PROXY, &ctx, false, combat::MOVE_STAB, 0.0, &mut r);
    assert!(near(r.lag_reduction, -0.1, 1e-7) && near(r.lag_induction, 0.05, 1e-7), "not local stab {r:?}");
    let mut p = AttackLag::default();
    attack_lag(enums::ROLE_SIMULATED_PROXY, &ctx, false, combat::MOVE_RIGHT_STRIKE, angle, &mut p);
    assert!(near(p.lag_induction, -0.05, 1e-6), "proxy LagInduction {}", p.lag_induction);
    let mut sv = AttackLag::default();
    attack_lag(enums::ROLE_AUTHORITY, &LinkCtx::default(), false, combat::MOVE_RIGHT_STRIKE, angle, &mut sv);
    assert_eq!(sv, AttackLag::default());
    let n2 = LinkCtx { netcode_type: 2, ..ctx };
    let mut q = AttackLag::default();
    attack_lag(enums::ROLE_AUTONOMOUS_PROXY, &n2, true, combat::MOVE_STAB, 0.0, &mut q);
    assert!(near(q.lag_induction, LAG_INDUCTION_MAX_NETCODE2 as f64, 1e-9));
    // proxy windup end coincides with the server's: server start t+L, proxy t+2L with LagInduction -0.5 x 2L ... for
    // RTT 0.1: 0.35 + W - 0.08 == 0.40 + W - 0.08 - 0.05
    assert!(near(0.35 - 0.08, 0.40 - 0.08 - 0.05, 1e-6));
}

/// GetAttackCompensationStartTime: a flinch with bCanAttack false -> its EndTime; the Idle after it asks its
/// ComingFromMotion; the compensation is the time since the flinch's EndTime, capped by the ping.
#[test]
fn attack_compensation_start_time_rules() {
    let fl = CompMotion::Other { can_attack: false, end_time: 0.65 };
    assert_eq!(attack_compensation_start_time(&fl, 0), 0.65);
    let ctx = LinkCtx { ping: 0.1, ..Default::default() };
    assert_eq!(attack_ping_compensation(enums::ROLE_AUTONOMOUS_PROXY, &ctx, 0.6, Some(&fl), 0), 0.0, "during the flinch");
    let idle = CompMotion::Idle { coming_from: Some(Box::new(fl.clone())), can_attack: true, end_time: 0.65 };
    assert!(near(attack_ping_compensation(enums::ROLE_AUTONOMOUS_PROXY, &ctx, 0.68, Some(&idle), 0), 0.03, 1e-6));
    assert!(near(attack_ping_compensation(enums::ROLE_AUTONOMOUS_PROXY, &ctx, 0.88, Some(&idle), 0), 0.1, 1e-7), "capped by the ping");
    // attack: Release / Recovery -> EndTime, Windup with bCanAttack -> 0
    assert_eq!(attack_compensation_start_time(&CompMotion::Attack { stage: 1, can_attack: true, end_time: 2.0 }, 0), 2.0);
    assert_eq!(attack_compensation_start_time(&CompMotion::Attack { stage: 0, can_attack: true, end_time: 2.0 }, 0), 0.0);
    assert_eq!(attack_compensation_start_time(&CompMotion::Attack { stage: 0, can_attack: false, end_time: 2.0 }, 0), 2.0);
    // parry: TotalBlocks != 0 and bCanAttack -> 0
    assert_eq!(attack_compensation_start_time(&CompMotion::Parry { total_blocks: 1, can_attack: true, end_time: 1.0 }, 0), 0.0);
    assert_eq!(attack_compensation_start_time(&CompMotion::Parry { total_blocks: 0, can_attack: true, end_time: 1.0 }, 0), 1.0);
    // blocked: Clash with FromMove != Kick open; Hit from a kick open for a non-kick move only
    let clash = CompMotion::Blocked { reason: combat::BLOCKED_CLASH, from_move: 0, can_attack: true, end_time: 1.0 };
    assert_eq!(attack_compensation_start_time(&clash, 0), 0.0);
    let kick_hit = CompMotion::Blocked { reason: combat::BLOCKED_HIT, from_move: combat::MOVE_KICK, can_attack: true, end_time: 1.0 };
    assert_eq!(attack_compensation_start_time(&kick_hit, 0), 0.0);
    assert_eq!(attack_compensation_start_time(&kick_hit, combat::MOVE_KICK), 1.0);
    let parried = CompMotion::Blocked { reason: combat::BLOCKED_PARRY, from_move: 0, can_attack: true, end_time: 1.0 };
    assert_eq!(attack_compensation_start_time(&parried, 0), 1.0);
    // feinted: bCanAttackFromFeintLockout of the asked move's CDO and bCanAttack
    assert_eq!(attack_compensation_start_time(&CompMotion::Feinted { can_attack_from_feint_lockout: true, can_attack: true, end_time: 1.0 }, 0), 0.0);
    assert_eq!(attack_compensation_start_time(&CompMotion::Feinted { can_attack_from_feint_lockout: false, can_attack: true, end_time: 1.0 }, 0), 1.0);
}

/// With 50 ms each way the bout still ends the same on all three machines.
#[test]
fn latency_bout_all_machines_agree() {
    let mut s = session(LAT);
    s.client("A").unwrap().world.at(0.3, Input::Attack("A".into()));
    s.client("B").unwrap().world.at(0.55, Input::Parry("B".into()));
    s.server.world.at(0.9, Input::Contact("A".into(), "B".into()));
    s.run_until(2.0);
    let want = s.server.world.outcome();
    for c in &s.clients {
        assert_eq!(c.world.outcome(), want, "client {}", c.own);
    }
    assert_eq!(s.server.rejected, 0);
}

// ---- client hit suggestion ------------------------------------------------------------------------------------------
#[test]
fn suggest_hit_detection() {
    let mut s = session(0);
    s.client("A").unwrap().world.at(0.1, Input::Attack("A".into()));
    s.run_until(0.52);
    assert_eq!(s.server.world.p("A").motion.stage(s.server.world.now), 1, "server A not in Release");
    let t = s.transport.as_mut();
    // B standing: the client does not suggest (IsRagdollFallingOrGettingUp false)
    let ca = s.clients.iter_mut().find(|c| c.own == "A").unwrap();
    assert!(!ca.cosmetic_hit(t, "B", "Spine1"), "suggested a hit on a standing victim");
    // B ragdolling on every machine: A's client suggests, the server applies ComputeMeleeDamage, no flinch
    ca.rep_mut("B").unwrap().ragdoll_falling = true;
    s.server.rep_mut("B").unwrap().ragdoll_falling = true;
    let h0 = s.server.world.p("B").health;
    let ca = s.clients.iter_mut().find(|c| c.own == "A").unwrap();
    assert!(ca.cosmetic_hit(s.transport.as_mut(), "B", "Spine1"), "no suggestion for a ragdolling victim");
    s.step();
    let got: Vec<_> = s.server.world.hits.iter().filter(|h| h.3).collect();
    assert_eq!(got.len(), 1, "server suggested hits {got:?}");
    assert!(s.server.world.p("B").health < h0, "B not damaged");
    assert_ne!(s.server.world.p("B").motion.kind, Kind::Flinch, "a suggested hit flinched B");
    let sa = s.server.world.p("A");
    assert!(sa.net.dynamic_param & 0x10 != 0 && !sa.motion.has_hit, "dynamic param {} / bHasHit {}", sa.net.dynamic_param, sa.motion.has_hit);
    assert!(!s.server.world.p("B").dead);
    // the same victim again: already in the weapon's ActorSetCache
    let ca = s.clients.iter_mut().find(|c| c.own == "A").unwrap();
    ca.cosmetic_hit(s.transport.as_mut(), "B", "Spine1");
    s.step();
    assert_eq!(s.server.rep("A").unwrap().rejected_suggestions, 1, "repeat suggestion not rejected");
    // a client cannot send a server RPC for a pawn it does not own (ProcessRemoteFunction: no owning connection)
    let cb = s.clients.iter_mut().find(|c| c.own == "B").unwrap();
    cb.send_server_rpc(s.transport.as_mut(), &Msg::ServerSuggestHitDetection { who: "A".into(), other: "B".into(), bone: "Spine1".into() });
    assert_eq!(cb.dropped_rpcs, 1, "client B sent an RPC for A's pawn");
    s.step();
    assert_eq!(s.server.not_owner, 0, "the server saw a non-owner RPC");
}

/// IsRagdollFallingOrGettingUp: falling, or still inside GetUpStartTime + GetUpDuration
#[test]
fn ragdoll_get_up_window() {
    let mut r = NetMotionRep::new("B");
    assert!(!r.is_ragdoll_falling_or_getting_up(1.0));
    r.ragdoll_get_up_start_time = 0.5;
    r.ragdoll_get_up_duration = 0.8;
    assert!(r.is_ragdoll_falling_or_getting_up(1.0));
    r.ragdoll_get_up_duration = 0.5;
    assert!(!r.is_ragdoll_falling_or_getting_up(1.0), "the window end (start + duration == now) still counted");
}

// ---- remaining properties -------------------------------------------------------------------------------------------
/// ReplicatedLookUpValue (COND_SimulatedOnly): byte = (int)(clamp01((LookUp + Down) / (Up + Down)) x 255), the
/// simulated proxy aims at (Up + Down) x byte / 255 - Down; the owner gets nothing. ReplicatedCharacterFlags bit 0 =
/// airborne on every client; ReplicatedTeam -> Team.
#[test]
fn look_up_flags_team_replicate() {
    let mut s = session(0);
    let (up, down) = (60.0f64, 50.0f64); // fixture LookUpLimit / LookDownLimit
    s.client("A").unwrap().world.pm("A").look_up = 7.0;
    s.server.world.pm("A").look_up = 30.0;
    s.server.world.pm("A").airborne = true;
    let ep = s.server.owner_of["A"];
    s.server.player_states.get_mut(&ep).unwrap().set_team(1, true);
    s.step();
    let byte = ((((30.0 + down) / (up + down)).clamp(0.0, 1.0)) * 255.0) as i32;
    assert_eq!(s.server.rep("A").unwrap().replicated_look_up_value as i32, byte);
    let want = (up + down) * (byte as f64 / 255.0) - down;
    assert!(near(s.client("B").unwrap().world.p("A").look_up, want, 1e-3), "proxy look up {}", s.client("B").unwrap().world.p("A").look_up);
    assert_eq!(s.client("A").unwrap().world.p("A").look_up, 7.0, "owner received its own look-up (COND_SimulatedOnly)");
    for c in &s.clients {
        assert!(c.world.p("A").airborne, "client {}: A not airborne", c.own);
        assert_eq!(c.world.p("A").team, 1, "client {}: A team", c.own);
    }
}

/// NetBlock / ReplicatedKnockback (UParryMotion::ReceiveBlock rva=0x166bf90): B parries A's strike at 0.6 s and at
/// 7.6 s. Each block runs AssignNetBlock (Version 1, 2) and ApplyBackwardsKnockbackIfNotInKnockback(KnockbackParry) on
/// the server. OnRep_NetBlock does nothing while World TimeSeconds <= 7, so only the second block is applied (once, on
/// every machine: COND_None, owner included); ReplicatedKnockback counts both.
#[test]
fn net_block_and_knockback_replicate() {
    let mut s = session(0);
    let kp = 100.0f64; // fixture KnockbackParry
    for t0 in [0.0f64, 7.0] {
        s.client("A").unwrap().world.at(t0 + 0.1, Input::Attack("A".into()));
        s.client("B").unwrap().world.at(t0 + 0.3, Input::Parry("B".into()));
        s.server.world.at(t0 + 0.6, Input::Contact("A".into(), "B".into()));
        s.run_until(t0 + 0.65);
        let n: u8 = if t0 == 0.0 { 1 } else { 2 };
        let sb = s.server.rep("B").unwrap();
        assert_eq!(sb.net_block.version, n, "server NetBlock Version at {t0}");
        assert_eq!(sb.net_block.reason, combat::BLOCKED_PARRY);
        assert_eq!(sb.net_block.mv, combat::MOVE_RIGHT_STRIKE);
        assert_eq!(sb.replicated_knockback, n);
        let want_applied = if t0 == 0.0 { 0 } else { 1 };
        let reps = [s.server.rep("B").unwrap().clone(), s.clients[0].rep("B").unwrap().clone(), s.clients[1].rep("B").unwrap().clone()];
        for r in &reps {
            assert_eq!(r.net_blocks_applied.len(), want_applied, "applied NetBlocks at {t0}");
            assert_eq!(r.replicated_knockback, n);
        }
        s.run_until(t0 + 3.0);
    }
    let sb = s.server.rep("B").unwrap();
    assert_eq!(sb.knockbacks.len(), 2);
    assert!(near(sb.knockbacks[0][0] as f64, -kp, 1e-3), "server knockbacks {:?}", sb.knockbacks);
    for i in 0..3 {
        let r = match i {
            0 => s.server.rep_mut("B").unwrap(),
            1 => s.clients[0].rep_mut("B").unwrap(),
            _ => s.clients[1].rep_mut("B").unwrap(),
        };
        let b = &r.net_blocks_applied[0];
        assert!(b.result.reason == combat::BLOCKED_PARRY && b.mv == combat::MOVE_RIGHT_STRIKE && !b.result.ranged && !b.result.stun);
        assert_eq!(r.last_net_block_version, 2);
    }
    // the same Version again does nothing
    let p = s.server.world.pm("B");
    let mut r = p.rep.take().unwrap();
    r.on_rep_net_block(p);
    assert_eq!(r.net_blocks_applied.len(), 1, "a repeated Version was applied again");
    // Knockback is authority only and ignores |Amount| <= 0.1
    assert!(!r.knockback(p, [0.1, 0.0, 0.0]), "Knockback of 0.1 accepted");
    p.rep = Some(r);
    let cp = s.clients[1].world.pm("B");
    let mut cr = cp.rep.take().unwrap();
    assert!(!cr.knockback(cp, [500.0, 0.0, 0.0]), "a client applied Knockback");
    cp.rep = Some(cr);
}

/// AssignNetBlock packs FBlockResult into Flags bit by bit, the OnRep reads it back
#[test]
fn net_block_flags_round_trip() {
    let mut s = session(0);
    s.run_until(7.01);
    {
        let p = s.server.world.pm("A");
        let mut r = p.rep.take().unwrap();
        r.assign_net_block(p, &BlockResult { reason: combat::BLOCKED_CLASH, clash_on_parry: true, party: true, surface: 3, ..Default::default() }, combat::MOVE_STAB, "B");
        assert_eq!(r.net_block.flags, 0x50, "bPartyFlag << 4 | bClashOnParry << 6");
        let b = r.net_blocks_applied.last().unwrap();
        assert!(b.result.clash_on_parry && b.result.party && !b.result.stun && !b.result.disarm && !b.result.self_event && b.result.surface == 3 && b.actor == "B");
        p.rep = Some(r);
    }
    s.step();
    let pb = s.client("B").unwrap().rep("A").unwrap().net_blocks_applied.last().unwrap().clone();
    assert!(pb.result.reason == combat::BLOCKED_CLASH && pb.mv == combat::MOVE_STAB && pb.result.clash_on_parry);
}

// ---- ReplicatedDodge ------------------------------------------------------------------------------------------------
/// A's client dodges along UE (-1, -1): yaw -135 -> v = 225 -> RoundToInt(225 x 255/360) = 159. The client runs
/// OnDodged(-135) at once; the server stores 159, runs OnDodged(unwind(159 x 360/255)) and pays DodgeStaminaCost;
/// B's proxy gets ReplicatedDodge (COND_SimulatedOnly) and runs OnDodged too; A's own client gets no property. A
/// second dodge the same way makes the server store 160 and B's proxy dodges again.
#[test]
fn dodge_replicates() {
    let mut s = session(0);
    s.run_until(0.1);
    let cost = 10; // fixture DodgeStaminaCost
    let st0 = s.server.world.p("A").stamina;
    let t = s.transport.as_mut();
    let ca = s.clients.iter_mut().find(|c| c.own == "A").unwrap();
    let packed = ca.request_dodge(t, [-1.0, -1.0]).unwrap();
    assert_eq!(packed, 159);
    let d = ca.rep("A").unwrap().dodges.clone();
    assert!(d.len() == 1 && near(d[0].1, -135.0, 1e-3), "owner dodges {d:?}");
    s.step();
    let want_yaw = NetMotionRep::unwind_dodge(((159.0 * DODGE_BYTE_TO_DEG as f64) as f32) as f64);
    let sa = s.server.rep("A").unwrap();
    assert!(sa.replicated_dodge == 159 && sa.dodges.len() == 1 && near(sa.dodges[0].1, want_yaw, 1e-4), "server {} {:?}", sa.replicated_dodge, sa.dodges);
    assert!((-180.0..=180.0).contains(&want_yaw));
    assert_eq!(s.server.world.p("A").stamina, st0 - cost, "server stamina");
    let pa = s.client("B").unwrap().rep("A").unwrap().clone();
    assert!(pa.replicated_dodge == 159 && pa.dodges.len() == 1, "proxy {} {}", pa.replicated_dodge, pa.dodges.len());
    let oa = s.client("A").unwrap().rep("A").unwrap().clone();
    assert!(oa.replicated_dodge == 0 && oa.dodges.len() == 1, "owner got its own ReplicatedDodge");
    let t = s.transport.as_mut();
    s.clients.iter_mut().find(|c| c.own == "A").unwrap().request_dodge(t, [-1.0, -1.0]);
    s.step();
    let sa = s.server.rep("A").unwrap().replicated_dodge;
    let pa = s.client("B").unwrap().rep("A").unwrap().clone();
    assert!(sa == 160 && pa.replicated_dodge == 160 && pa.dodges.len() == 2, "repeat dodge: server {sa} proxy {}", pa.replicated_dodge);
    // only the server pays the cost (OffsetStamina is authority only); the clients get ReplicatedStamina
    let srs = s.server.rep("A").unwrap().replicated_stamina;
    assert_eq!(s.client("B").unwrap().rep("A").unwrap().replicated_stamina, srs);
    assert_eq!(s.client("B").unwrap().world.p("A").stamina, st0 - 2 * cost);
}

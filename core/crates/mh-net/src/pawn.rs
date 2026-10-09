//! The seam between the replication rules and one machine's copy of a character (the combat crate's MotionSystem).
//!
//! The GDScript port couples them both ways: MotionSystem calls `net_rep.*` hooks (assign, init_motion, the stat
//! writes, attack lag), and NetMotionRep reads/writes `sys.*`. In Rust the dependency runs one way (mh-net ->
//! mordhau-core), so:
//!   - `Pawn` is what the rules read and write on the character; the combat crate implements it for its
//!     MotionSystem (and the test fixture world in tests/support does the same).
//!   - `NetSlot` is the small net state the character itself must carry, because the combat code reads it while a
//!     motion is being created (UMordhauMotion bInitiatedLocally +0x58, bWasConfirmedByAuthority +0x59,
//!     ExpectedDelay +0x48, set by HandleNetMotionUpdate before Initialize; the machine's ping / TimeDilation /
//!     CVars the attack's OnBegin reads). It lives in the character, not in the replication state, so a motion
//!     created while the replication state is borrowed still sees it.
//!   - `NetMotionRep` (rep.rs) is the replication state; the character stores it as
//!     `Option<Box<NetMotionRep>>` (`Pawn::rep_slot`) and the combat hooks take it out, call it, and put it back
//!     (see `with_rep`).
//! Contract for an implementor: every `ChangeMotion_Internal` (MotionSystem._change) calls
//! `slot_mut().init_motion(serial)` with a fresh serial for the new motion object and writes the returned
//! ExpectedDelay on it before OnBegin; `set_health_internal` / `set_stamina_internal` / `offset_stamina_raw` change
//! the stat only (the rules write the replicated bytes themselves).

use crate::consts::*;
use crate::enums::{self, combat};
use crate::netmotion::FNetMotion;
use crate::rep::NetMotionRep;
use crate::ue::{f32r, maxf, minf};
pub use mordhau_core::combat::MotionId;

/// The machine settings the ping rules read and the per-character net data combat reads while creating a motion:
/// defined in mordhau-core's replication seam (combat/net.rs, agreed with rust-combat) so the combat World carries
/// them; re-exported here.
pub use mordhau_core::combat::net::{NetCtx, NetSlot};
pub type LinkCtx = NetCtx;

/// One machine's copy of a character, as the net rules see it.
pub trait Pawn {
    fn name(&self) -> &str;
    /// AActor Role on this machine (enums::ROLE_*)
    fn role(&self) -> u8;
    /// AAdvancedCharacter +0x504 bIsDead
    fn is_dead(&self) -> bool;
    /// World TimeSeconds of this machine
    fn now(&self) -> f64;
    /// UMotionSystemComponent +0xc4 NetMotion
    fn net(&self) -> FNetMotion;
    fn set_net(&mut self, nm: FNetMotion);
    fn slot(&self) -> &NetSlot;
    fn slot_mut(&mut self) -> &mut NetSlot;
    /// the replication state stored on this character (None without net); taken out while a rule runs
    fn take_rep(&mut self) -> Option<Box<NetMotionRep>>;
    fn put_rep(&mut self, r: Box<NetMotionRep>);
    fn rep_ref(&self) -> Option<&NetMotionRep>;
    /// Motion (+0xe8), None = null
    fn motion(&self) -> Option<MotionId>;
    fn has_motion(&self) -> bool {
        self.motion().is_some()
    }
    /// The motion-creating tail of HandleNetMotionUpdate for the current `net()` (MotionSystem.begin_net_motion):
    /// NewObject of the class for MotionType, ChangeMotion_Internal, DynamicParamChanged(0, DynamicParam) when not 0.
    fn begin_net_motion(&mut self);
    /// Motion->DynamicParamChanged(old, new) (UAttackMotion::OnDynamicParamChanged_Implementation rva=0x1631060 etc.)
    fn on_dynamic_param_changed(&mut self, old: u8, new: u8);
    fn trace_event(&mut self, _s: &str) {}
    /// the machine's float model for the rules' arithmetic that the reference keeps in doubles: identity (the GDScript
    /// reference, mordhau-core Precision::Reference) or binary32 rounding (the exe, Precision::Exe: World::qf)
    fn q(&self) -> fn(f64) -> f64 {
        |x| x
    }

    // stats (UStatComponent StatValue +0xb0, Min +0xbc, Max +0xc0)
    fn health(&self) -> i64;
    fn health_range(&self) -> (i64, i64);
    /// SetStatValue_Internal(v, bReplicate false): OnDied when it goes from > 0 to < 1 (UHealthStatComponent::
    /// SetStatValue_Internal rva=0x14d5cf0)
    fn set_health_internal(&mut self, v: i64);
    fn stamina(&self) -> i64;
    fn stamina_range(&self) -> (i64, i64);
    fn set_stamina_internal(&mut self, v: i64);
    /// UStaminaStatComponent::OffsetStamina rva=0x1507c20 without the replicated write: returns true when the stat
    /// changed (it does nothing off the authority)
    fn offset_stamina_raw(&mut self, v: i64) -> bool;
    /// UStatComponent::StopRegeneration
    fn stop_stamina_regen(&mut self, t: f64);

    // character fields the replicated properties carry
    /// ReplicatedCharacterFlags bit 0 (AAdvancedCharacter::IsAirborne rva=0x14873e0)
    fn airborne(&self) -> bool;
    fn set_airborne(&mut self, b: bool);
    /// AAdvancedCharacter +0x520 LookUpValue
    fn look_up_value(&self) -> f64;
    fn set_look_up_value(&mut self, v: f64);
    /// (LookUpLimit, LookDownLimit) class defaults
    fn look_limits(&self) -> (f64, f64);
    /// AMordhauCharacter DodgeStaminaCost
    fn dodge_stamina_cost(&self) -> i64;
    /// AMordhauCharacter KnockbackParry
    fn knockback_parry(&self) -> f64;
    /// the root component's forward in UE axes; None -> FVector::ForwardVector
    fn forward(&self) -> Option<[f32; 3]>;
    /// MotionSystem.team (combat team convention, 255 = none)
    fn set_team(&mut self, team: i64);
}


/// a borrowed pawn is a pawn (NetWorld::P<'a> = &'a mut ToyPawn in tests)
impl<T: Pawn + ?Sized> Pawn for &mut T {
    fn q(&self) -> fn(f64) -> f64 {
        (**self).q()
    }
    fn trace_event(&mut self, s: &str) {
        (**self).trace_event(s)
    }
    fn name(&self) -> &str {
        (**self).name()
    }
    fn role(&self) -> u8 {
        (**self).role()
    }
    fn is_dead(&self) -> bool {
        (**self).is_dead()
    }
    fn now(&self) -> f64 {
        (**self).now()
    }
    fn net(&self) -> FNetMotion {
        (**self).net()
    }
    fn set_net(&mut self, nm: FNetMotion) {
        (**self).set_net(nm)
    }
    fn slot(&self) -> &NetSlot {
        (**self).slot()
    }
    fn slot_mut(&mut self) -> &mut NetSlot {
        (**self).slot_mut()
    }
    fn take_rep(&mut self) -> Option<Box<NetMotionRep>> {
        (**self).take_rep()
    }
    fn put_rep(&mut self, r: Box<NetMotionRep>) {
        (**self).put_rep(r)
    }
    fn rep_ref(&self) -> Option<&NetMotionRep> {
        (**self).rep_ref()
    }
    fn motion(&self) -> Option<MotionId> {
        (**self).motion()
    }
    fn begin_net_motion(&mut self) {
        (**self).begin_net_motion()
    }
    fn on_dynamic_param_changed(&mut self, old: u8, new: u8) {
        (**self).on_dynamic_param_changed(old, new)
    }
    fn health(&self) -> i64 {
        (**self).health()
    }
    fn health_range(&self) -> (i64, i64) {
        (**self).health_range()
    }
    fn set_health_internal(&mut self, v: i64) {
        (**self).set_health_internal(v)
    }
    fn stamina(&self) -> i64 {
        (**self).stamina()
    }
    fn stamina_range(&self) -> (i64, i64) {
        (**self).stamina_range()
    }
    fn set_stamina_internal(&mut self, v: i64) {
        (**self).set_stamina_internal(v)
    }
    fn offset_stamina_raw(&mut self, v: i64) -> bool {
        (**self).offset_stamina_raw(v)
    }
    fn stop_stamina_regen(&mut self, t: f64) {
        (**self).stop_stamina_regen(t)
    }
    fn airborne(&self) -> bool {
        (**self).airborne()
    }
    fn set_airborne(&mut self, b: bool) {
        (**self).set_airborne(b)
    }
    fn look_up_value(&self) -> f64 {
        (**self).look_up_value()
    }
    fn set_look_up_value(&mut self, v: f64) {
        (**self).set_look_up_value(v)
    }
    fn look_limits(&self) -> (f64, f64) {
        (**self).look_limits()
    }
    fn dodge_stamina_cost(&self) -> i64 {
        (**self).dodge_stamina_cost()
    }
    fn knockback_parry(&self) -> f64 {
        (**self).knockback_parry()
    }
    fn forward(&self) -> Option<[f32; 3]> {
        (**self).forward()
    }
    fn set_team(&mut self, team: i64) {
        (**self).set_team(team)
    }
}

/// Take the pawn's replication state out, run `f`, put it back (the combat hooks' pattern). None without net.
pub fn with_rep<P: Pawn + ?Sized, R>(p: &mut P, f: impl FnOnce(&mut NetMotionRep, &mut P) -> R) -> Option<R> {
    let mut rep = p.take_rep()?;
    let r = f(&mut rep, p);
    p.put_rep(rep);
    Some(r)
}

pub fn is_confirmed<P: Pawn + ?Sized>(p: &P) -> bool {
    p.slot().is_confirmed(p.motion())
}

pub fn is_initiated_locally<P: Pawn + ?Sized>(p: &P) -> bool {
    p.slot().is_initiated_locally(p.motion())
}

// ---- attack ping compensation ------------------------------------------------------------------------------------

/// The current motion as the virtual UMordhauMotion::GetAttackCompensationStartTime(Move) of each class reads it
/// (the combat crate builds it from its motion object).
#[derive(Clone, Debug)]
pub enum CompMotion {
    /// UAttackMotion: Stage, bCanAttack, EndTime
    Attack { stage: u8, can_attack: bool, end_time: f64 },
    /// UParryMotion: TotalBlocks, bCanAttack, EndTime
    Parry { total_blocks: i32, can_attack: bool, end_time: f64 },
    /// UIdleMotion: ComingFromMotion (None = null), bCanAttack, EndTime
    Idle { coming_from: Option<Box<CompMotion>>, can_attack: bool, end_time: f64 },
    /// UBlockedMotion: Reason, FromMove, bCanAttack, EndTime
    Blocked { reason: u8, from_move: u8, can_attack: bool, end_time: f64 },
    /// UFeintedMotion: bCanAttackFromFeintLockout (+0xb7d) of the CDO of GetAttackMotionClass(Move) for the asked
    /// move (the combat crate resolves it), bCanAttack, EndTime
    Feinted { can_attack_from_feint_lockout: bool, can_attack: bool, end_time: f64 },
    /// any other UMordhauMotion (Flinch, Stun, ...): bCanAttack, EndTime
    Other { can_attack: bool, end_time: f64 },
}

/// EAttackStage (UHT name table "EAttackStage::Windup" @file 0x43882c0, "::Release" 0x43882d8, "::Recovery" 0x43882f0)
pub const STAGE_RELEASE: u8 = 1;
pub const STAGE_RECOVERY: u8 = 2;

/// The virtual UMordhauMotion::GetAttackCompensationStartTime(Move):
///   UMordhauMotion rva=0x165c630 (Flinch and others): !bCanAttack -> EndTime, else 0
///   UAttackMotion rva=0x161fc90: Stage Release or Recovery (Stage - 1 < 2) -> EndTime; !bCanAttack -> EndTime; else 0
///   UParryMotion rva=0x165c640: TotalBlocks != 0 and bCanAttack -> 0, else EndTime
///   UIdleMotion rva=0x165c600: ComingFromMotion != null -> its GetAttackCompensationStartTime(Move); else the base rule
///   UBlockedMotion rva=0x165c460: Clash with FromMove != Kick, or Hit with FromMove == Kick and Move != Kick:
///     bCanAttack -> 0, else EndTime; every other case -> EndTime
///   UFeintedMotion rva=0x165c4a0: the CDO of GetAttackMotionClass(Move) is an attack with bCanAttackFromFeintLockout
///     (+0xb7d) and this motion's bCanAttack -> 0, else EndTime
pub fn attack_compensation_start_time(m: &CompMotion, mv: u8) -> f64 {
    match m {
        CompMotion::Attack { stage, can_attack, end_time } => {
            if *stage == STAGE_RELEASE || *stage == STAGE_RECOVERY || !can_attack {
                *end_time
            } else {
                0.0
            }
        }
        CompMotion::Parry { total_blocks, can_attack, end_time } => {
            if *total_blocks != 0 && *can_attack {
                0.0
            } else {
                *end_time
            }
        }
        CompMotion::Idle { coming_from: Some(c), .. } => attack_compensation_start_time(c, mv),
        CompMotion::Idle { coming_from: None, can_attack, end_time } | CompMotion::Other { can_attack, end_time } => {
            if !can_attack {
                *end_time
            } else {
                0.0
            }
        }
        CompMotion::Blocked { reason, from_move, can_attack, end_time } => {
            let kick = combat::MOVE_KICK;
            let open = if *reason == combat::BLOCKED_CLASH {
                *from_move != kick
            } else {
                *reason == combat::BLOCKED_HIT && *from_move == kick && mv != kick
            };
            if open && *can_attack {
                0.0
            } else {
                *end_time
            }
        }
        CompMotion::Feinted { can_attack_from_feint_lockout, can_attack, end_time } => {
            if *can_attack_from_feint_lockout && *can_attack {
                0.0
            } else {
                *end_time
            }
        }
    }
}

/// The ping half of UMotionSystemComponent::AssignNetAttackMotion rva=0x14b3210 (the caller clamps the result to
/// [0, max_ping_compensation] and packs it into Param2):
///   ping = GetControllerRoleIncludingVehicle == 2 ? GetPing(bUseMedian) x TimeDilation : 0
///   Motion != null: min(ping, max(0, now - Motion->GetAttackCompensationStartTime(Move))); else ping
pub fn attack_ping_compensation(role: u8, ctx: &NetCtx, now: f64, motion: Option<&CompMotion>, mv: u8) -> f64 {
    let mut ping = 0.0f64;
    if role == enums::ROLE_AUTONOMOUS_PROXY {
        ping = f32r(ctx.ping * ctx.time_dilation);
    }
    let Some(m) = motion else { return ping };
    let since = maxf(f32r(now - attack_compensation_start_time(m, mv)), 0.0);
    minf(since, ping)
}

/// The attack fields UAttackMotion::OnBegin_Implementation's lag rules write.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct AttackLag {
    /// +LagReduction
    pub lag_reduction: f64,
    /// +LagInduction
    pub lag_induction: f64,
    /// AttackInfo MissRecovery
    pub miss_recovery: f64,
}

/// AttackMotion hook (OnBegin, before Windup = LagReduction + LagInduction + Windup), from UAttackMotion::
/// OnBegin_Implementation rva=0x162eda0 (disassembled 0x14162f3e9..0x14162f61d):
///   EstimatedNetworkDelay (+0x10a0) = the local player controller's AMordhauPlayerState PingMedian (else ExactPing x
///     0.001; 0 with no local player controller, i.e. on a dedicated server) x WorldSettings TimeDilation
///   owner GetControllerRoleIncludingVehicle == 2 (the owning client):
///     !bInitiatedLocally -> LagReduction -= EstimatedNetworkDelay
///     on foot, by the m.NetcodeType CVar: 2 -> LagInduction = min(END, 0.025); 1 -> unchanged; else LagInduction =
///       min(END, 0.15), x 0.5 for Stab / AltStab (Move 2, 3), x (1 - |AngleTarget| x 0.5) for the strikes (Move 0, 1)
///     MissRecovery = max(MissRecovery - LagInduction, 0)
///   else owner Role == 1 (a simulated proxy): f = m.PingExtrapolationFactor (default 0.5); f in (0, 1] ->
///     LagInduction -= f x EstimatedNetworkDelay
/// `initiated_locally`: is_initiated_locally of the attack being begun (its NetSlot flags are already set).
/// Not ported: the camera look / location lag-induction targets the owning client sets later in OnBegin (cosmetic,
/// `UAttackMotion::OnBegin_Implementation` at 0x14163006b..0x1416300d6). Vehicles: none in this world.
/// Rounding: the f32() points of the reference (net_motion_rep.gd attack_lag).
pub fn attack_lag(role: u8, ctx: &NetCtx, initiated_locally: bool, mv: u8, angle_target: f64, lag: &mut AttackLag) {
    let est = f32r(ctx.ping * ctx.time_dilation);
    if role == enums::ROLE_AUTONOMOUS_PROXY {
        if !initiated_locally {
            lag.lag_reduction = f32r(lag.lag_reduction - est);
        }
        match ctx.netcode_type {
            2 => lag.lag_induction = minf(est, LAG_INDUCTION_MAX_NETCODE2 as f64),
            1 => {}
            _ => {
                let li = minf(est, LAG_INDUCTION_MAX as f64);
                lag.lag_induction = li;
                if mv == combat::MOVE_STAB || mv == combat::MOVE_ALT_STAB {
                    lag.lag_induction = f32r(li * LAG_INDUCTION_STAB_FACTOR as f64);
                } else if mv == combat::MOVE_RIGHT_STRIKE || mv == combat::MOVE_LEFT_STRIKE {
                    let k = f32r(LAG_UNIT as f64 - f32r(angle_target.abs() * LAG_INDUCTION_STAB_FACTOR as f64));
                    lag.lag_induction = f32r(k * li);
                }
            }
        }
        lag.miss_recovery = maxf(f32r(lag.miss_recovery - lag.lag_induction), 0.0);
    } else if role == enums::ROLE_SIMULATED_PROXY {
        let mut f = ctx.ping_extrapolation_factor;
        if f >= 0.0 {
            f = minf(f, LAG_UNIT as f64);
            if f > 0.0 {
                lag.lag_induction = f32r(lag.lag_induction - f32r(f * est));
            }
        }
    }
}

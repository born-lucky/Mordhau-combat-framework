//! What a machine's simulation must offer the session layer. The GDScript port calls CombatState directly; here it is
//! a trait so the same server / client / session code drives mordhau-core's combat World (core_world.rs) and the
//! small test fixture (tests/support). A pawn is reached as a short-lived view (`net_pawn`), because in the combat
//! World a fighter is an index into the world, not an object.

use crate::netmotion::FNetMotion;
use crate::pawn::{NetCtx, Pawn};
use crate::rep::NetMotionRep;

/// The attacker's LastAttackMotion as AMordhauCharacter::ServerSuggestHitDetection_Implementation rva=0x1567d80 reads
/// it after the victim gate (see server.rs)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LastAttackView {
    /// LastAttackMotion is still the current motion
    pub is_current: bool,
    /// its Stage == Recovery
    pub in_recovery: bool,
    /// AttackInfo.bStopOnHit (+0xed0)
    pub stop_on_hit: bool,
    /// the victim's bWillStopMelee (+0x9b1)
    pub victim_will_stop_melee: bool,
}

/// A pawn's current UClimbingMotion as the movement side (mh-character exe_climb) needs it: `key` tells one climb from
/// the next (motion slot, StartTime bits), `params` are the NetMotion params OnBegin_Implementation rva=0x165e550 reads
/// (+0xc6..+0xc9), `data` the class timings (UClimbingMotion ctor rva=0x16457d0; OnBegin swaps the slow ones in)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClimbView {
    pub key: (u32, u32),
    pub start_time: f32,
    pub params: [u8; 4],
    pub data: mh_character::exe_climb::ClimbMotionData,
}

/// what the rules read of another pawn without borrowing it mutably
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PawnInfo {
    pub role: u8,
    pub dead: bool,
    pub now: f64,
    pub net: FNetMotion,
}

pub trait NetWorld {
    type P<'a>: Pawn
    where
        Self: 'a;

    fn now(&self) -> f64;
    fn tick_n(&self) -> u64;
    /// a new client world starts on the server's clock (NetSession.add_client)
    fn set_clock(&mut self, tick_n: u64, now: f64);
    /// CombatState.authority: false on a client's copy (clients process no hits); also installs the world's net hooks
    fn set_authority(&mut self, authority: bool);

    /// add a fighter with its role on this machine (enums::ROLE_*) and whether, on the authority, its controller is on
    /// a remote client (MotionSystem.remote_controlled); `rep` = its replication state, `ctx` its machine settings
    fn add_fighter(&mut self, name: &str, weapon: &str, left: &str, role: u8, remote_controlled: bool, rep: NetMotionRep, ctx: NetCtx);
    fn remove_fighter(&mut self, name: &str);
    /// a view of one pawn for the rules (None: no such fighter)
    fn net_pawn<'a>(&'a mut self, name: &str) -> Option<Self::P<'a>>;
    fn pawn_info(&self, name: &str) -> Option<PawnInfo>;
    fn rep(&self, name: &str) -> Option<&NetMotionRep>;
    fn rep_mut(&mut self, name: &str) -> Option<&mut NetMotionRep>;
    /// set a fighter's machine settings (the ping changes every client frame)
    fn set_ctx(&mut self, name: &str, ctx: NetCtx);
    /// the fighter's combat team (MotionSystem.team, 255 = none)
    fn set_team(&mut self, name: &str, team: i64);
    /// fighters in add order
    fn fighter_names(&self) -> Vec<String>;

    /// One fixed step: advance the clock, run what is scheduled for it (the world's own scheduled calls first, then
    /// `call_phase`: the server's received RPCs, then the inputs), then the motions' tick.
    fn step(&mut self, call_phase: &mut dyn FnMut(&mut Self));

    /// ServerSuggestHitDetection_Implementation, the weapon half: the attacker's MotionSystem LastAttackMotion with a
    /// Weapon, and OtherCharacter not yet in the weapon's ActorSetCache -> add it (ActorIgnoreCache + ActorSetCache)
    /// and return what the rest of the rule reads; None = rejected
    fn suggest_hit_prepare(&mut self, attacker: &str, victim: &str) -> Option<LastAttackView>;
    /// the damage half: OtherCharacter->ComputeMeleeDamage(AttackInfo.Damage, HeadBonus, LegBonus, bone) (vcall
    /// +0x948 = AMordhauCharacter::ComputeMeleeDamage); OtherCharacter->TakeDamage(Damage, FMordhauDamageEvent,
    /// controller, this) (vcall +0x590). None of ProcessHitForDamage's extras (no damage factors, no stamina damage,
    /// no flinch, no hit stamina reward). The world records a "hit" event with suggested = true.
    fn suggest_hit_damage(&mut self, attacker: &str, victim: &str, bone: &str);
    /// FNetMotion::Blocked rva=0x1615f50 through the attacker's AssignNetMotion (MotionSystem.assign_net_blocked)
    fn assign_net_blocked(&mut self, who: &str, reason: u8, flags: u8, time_s: f64);
    /// AMordhauCharacter::ServerDropParry_Implementation rva=0x1567ad0 (MotionSystem.server_drop_parry_implementation)
    fn server_drop_parry_implementation(&mut self, who: &str, motion_id: u8);

    /// every death on this machine so far, in order (the combat "died" events; NetDuelMatch reads them)
    fn deaths(&self) -> Vec<String>;

    /// AMordhauCharacter::RequestClimb rva=0x1564590 (offset = ClimbTargetLocation - root location): FNetMotion::Climbing
    /// rva=0x14b5380 through AssignNetMotion. false: this world has no climbing (the test fixture)
    fn net_request_climb(&mut self, who: &str, offset: [f32; 3], slow: bool) -> bool {
        let _ = (who, offset, slow);
        false
    }
    /// `who`'s current motion when it is a UClimbingMotion
    fn net_climb_motion(&self, who: &str) -> Option<ClimbView> {
        let _ = who;
        None
    }
    /// BP_MordhauCharacter AttemptClimb's motion gate (mh-character `motion_blocks_climb`): the current motion is a
    /// UClimbingMotion, a UParryMotion, or a UAttackMotion not in Stage 2
    fn net_motion_blocks_climb(&self, who: &str) -> bool {
        let _ = who;
        false
    }

    /// A world that runs its own movement (mh-sim's Sim): the network host's state of `who` (a remote player's
    /// movement, from its ServerMoves) is copied in before the step and the world must not move it itself
    fn net_set_movement(&mut self, who: &str, m: &mh_character::ExeMovement) {
        let _ = (who, m);
    }
    /// such a world's movement of `who` (a server bot it moves), for the host to replicate
    fn net_movement(&self, who: &str) -> Option<mh_character::ExeMovement> {
        let _ = who;
        None
    }
    /// a server-controlled pawn was added (add_fighter with no owner): give it its AI, if the world has bots
    fn net_add_bot(&mut self, who: &str) {
        let _ = who;
    }
}

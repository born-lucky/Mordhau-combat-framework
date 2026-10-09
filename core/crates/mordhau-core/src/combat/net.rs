//! The replication seam (owner: rust-net, agreed with rust-combat): what the combat code calls when a machine is
//! networked, and the small per-fighter net data combat itself reads while a motion is created. Mirrors the
//! reference's `MotionSystem.net_rep` hooks (godot/game/combat/motion_system.gd _change / _assign /
//! assign_net_motion_dynamic_param / server_drop_parry / offset_stamina / _post_take_damage / assign_net_attack_motion,
//! attack_motion.gd OnBegin, parry_motion.gd receive_block). The rules themselves live in mh-net (NetMotionRep), which
//! implements `NetHooks`. Every hook is skipped when `World::net` is None (one machine, authority: golden traces
//! unchanged).
//!
//! Re-entrancy: the hooks object is stateless (an Rc cloned before each call, so `&mut World` stays free); mh-net keeps
//! its per-fighter replication state in `Fighter::net_state`. `NetSlot` is plain data in the fighter because combat
//! reads it while a motion is being created inside a hook call (UMordhauMotion bInitiatedLocally +0x58,
//! bWasConfirmedByAuthority +0x59, ExpectedDelay +0x48; the machine's ping / TimeDilation / CVars for
//! UAttackMotion::OnBegin).

use super::{MotionId, NetMotion, World};

/// The machine settings the ping rules read (NetServer / NetClient in the reference).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NetCtx {
    /// net mode (UNCONFIRMED names: 0 standalone, 1 dedicated server, 2 listen server, 3 client; mh-net enums)
    pub net_mode: i64,
    /// UMordhauUtilityLibrary::GetPing rva=0x1624d00 with bUseMedian (seconds); 0 on a dedicated server
    pub ping: f64,
    /// AWorldSettings +0x2e8 TimeDilation (1.0 from AWorldSettings::AWorldSettings rva=0x3576250)
    pub time_dilation: f64,
    /// m.NetcodeType CVar (default 0, `dynamic initializer for 'CVarNetcodeType'` rva=0x6407b0)
    pub netcode_type: i64,
    /// m.PingExtrapolationFactor CVar (default 0.5, .rdata 0x143fe4e04)
    pub ping_extrapolation_factor: f64,
}

impl Default for NetCtx {
    fn default() -> Self {
        NetCtx { net_mode: 0, ping: 0.0, time_dilation: 1.0, netcode_type: 0, ping_extrapolation_factor: 0.5 }
    }
}

/// bInitiatedLocally / bWasConfirmedByAuthority / ConfirmedByAuthorityTime of the CURRENT motion, the pending values
/// HandleNetMotionUpdate rva=0x14c01b0 writes on the motion it creates, and the machine ctx.
#[derive(Clone, Debug, Default)]
pub struct NetSlot {
    pub ctx: NetCtx,
    /// (bIsLocalSimulation, ExpectedDelay) for the one motion HandleNetMotionUpdate is creating
    pub pending: Option<(bool, f64)>,
    pub flags_motion: Option<MotionId>,
    pub confirmed: bool,
    pub initiated_locally: bool,
    pub confirmed_by_authority_time: f64,
}

impl NetSlot {
    /// MotionSystem._change hook before OnBegin (NetMotionRep.init_motion): a motion created by HandleNetMotionUpdate
    /// takes the pending flags and ExpectedDelay (0x1414c0668..0x1414c070a); any other (ChangeMotion: Idle after
    /// OnEnded) starts with both flags false and ExpectedDelay 0 (NewObject zero-fills). Returns the ExpectedDelay.
    pub fn init_motion(&mut self, m: MotionId) -> f64 {
        self.flags_motion = Some(m);
        match self.pending.take() {
            Some((local, d)) => {
                self.confirmed = !local;
                self.initiated_locally = local;
                d
            }
            None => {
                self.confirmed = false;
                self.initiated_locally = false;
                0.0
            }
        }
    }

    pub fn is_confirmed(&self, cur: Option<MotionId>) -> bool {
        cur.is_some() && self.flags_motion == cur && self.confirmed
    }

    pub fn is_initiated_locally(&self, cur: Option<MotionId>) -> bool {
        cur.is_some() && self.flags_motion == cur && self.initiated_locally
    }
}

/// Implemented by mh-net (NetMotionRep's rules). `fi`: the fighter index on this machine.
pub trait NetHooks {
    /// UMotionSystemComponent::AssignNetMotion rva=0x14b34c0, the role-dependent rest (nm.id already = net.id + 1);
    /// replaces the offline `net = nm; begin_net_motion`
    fn assign(&self, w: &mut World, fi: usize, nm: NetMotion);
    /// UMotionSystemComponent::AssignNetMotionDynamicParam rva=0x14b35f0 (replaces the offline body)
    fn assign_dynamic_param(&self, w: &mut World, fi: usize, v: i64);
    /// AMordhauCharacter::ServerDropParry as the RPC from a non-authority
    fn send_server_drop_parry(&self, w: &mut World, fi: usize, motion_id: i64);
    /// an authority stat change with bReplicate (WriteReplicatedStat): health true / stamina false
    fn stat_written(&self, w: &mut World, fi: usize, health: bool);
    /// AssignNetAttackMotion rva=0x14b3210, the ping half (combat clamps and packs it)
    fn attack_ping_compensation(&self, w: &World, fi: usize, mv: i64) -> f64;
    /// UAttackMotion::OnBegin_Implementation rva=0x162eda0 lag rules on attack `m` (LagReduction / LagInduction /
    /// MissRecovery), before Windup = LagReduction + LagInduction + Windup
    fn attack_lag(&self, w: &mut World, fi: usize, m: MotionId);
    /// UParryMotion::ReceiveBlock rva=0x166bf90: AssignNetBlock(Parry) + ApplyBackwardsKnockbackIfNotInKnockback
    fn receive_block(&self, w: &mut World, fi: usize, attacker_move: i64, shield_wall: bool);
}

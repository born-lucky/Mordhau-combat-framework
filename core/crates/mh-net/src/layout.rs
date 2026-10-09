//! Which character properties replicate, to whom, and what a client does when one arrives. Plain data plus two
//! functions; the server's shadow compare (NetServer::replicate) and the client apply (NetClient::receive) use it.
//! Port of godot/game/net/net_rep_layout.gd.
//!
//! Conditions, from the registration code (the FDoRepLifetimeParams first byte is the ELifetimeCondition, enums.rs):
//!   ReplicatedNetMotion  COND_SkipOwner  AMordhauCharacter::GetLifetimeReplicatedProps rva=0x153fd60: `mov qword
//!                        [rbp-0x20], 3` at 0x14153ffa5, registered at 0x141540008 (decomp AMordhauCharacter.cpp:
//!                        FName L"ReplicatedNetMotion"). The owner gets ClientSetNetMotion instead.
//!   ReplicatedStamina    COND_None       same function, zeroed params (FName L"ReplicatedStamina")
//!   ReplicatedHealth     COND_None       AAdvancedCharacter::GetLifetimeReplicatedProps rva=0x14846a0, zeroed params
//!   ReplicatedCharacterFlags COND_None   AAdvancedCharacter::GetLifetimeReplicatedProps rva=0x14846a0, zeroed params
//!   ReplicatedLookUpValue COND_SimulatedOnly  same function: params first byte 4 (`&DAT_00000004`) before it
//!   ReplicatedTeam       COND_None       AMordhauPlayerState::GetLifetimeReplicatedProps rva=0x15d7a90, zeroed params
//!                        (a player state property: kept with the pawn of the controller that owns it)
//!   NetBlock             COND_None       AMordhauCharacter::GetLifetimeReplicatedProps rva=0x153fd60: push-model
//!   ReplicatedKnockback  COND_None       FRepPropertyDescriptor, SharedParams first byte 0, bIsPushBased 1
//!   ReplicatedDodge      COND_SimulatedOnly  same function: params first byte 4 (`&DAT_00000004`) before FName
//!                        L"ReplicatedDodge" (the owner runs OnDodged itself when it requests the dodge)
//! Registered there and not modelled (no state for them in the duel world): ReplicatedVoiceCommand (COND_SkipOwner),
//! ReplicatedCustomizationReplicationActor, Quiver, NetDamage (AAdvancedCharacter, push-model), SpawnTurnValue; the
//! character's Team byte (+0x660, COND_None, OnRep_Team rva=0x1491ed0 only swaps gameplay tags). Equipment /
//! RightHandEquipment / LeftHandEquipment (COND_None) are actor references: the pawn's weapon paths travel in the
//! spawn message (NetServer::spawn_pawn). bIsHoldingBlock (+0xb99) is not replicated and no RPC carries it:
//! AMordhauCharacter::BlockPressed rva=0x1531650 / BlockReleased rva=0x15316a0 set it on the machine that has the
//! input, and the server learns a released held parry only through ServerDropParry. Movement (UCharacterMovementComponent
//! saved moves / ServerMove) is engine code (UE 4.26), engine-UNCONFIRMED, not ported.
//!
//! Engine rules (UE 4.26, statically linked; what the shipped exe can and cannot confirm):
//!   - This exe is a client build without server code: UNetDriver::ServerReplicateActors rva=0x7bf3a0 is 3 bytes,
//!     `xor eax, eax; ret` (the WITH_SERVER_CODE stub, folded with 11k other trivial functions), so the server's send
//!     rules (per-connection shadow compare, send only on change, priority / relevancy, RPC-before-properties order in
//!     a frame) cannot be read from it: engine-UNCONFIRMED, modelled as UE 4.26 documents them.
//!   - RPC sender gate, CONFIRMED: UNetDriver::ProcessRemoteFunction rva=0x323cf80 gets the actor's connection (vcall
//!     +0x4a8 = APawn::GetNetConnection for a character, at 0x14323d386) and with none logs "No owning connection for
//!     actor %s. Function %s will not be processed." and drops the call (0x14323d38f -> 0x14323d3d6). A client only
//!     has a connection for pawns it owns, so it can only send server RPCs for its own pawn (NetClient).
//!   - RPC receiver: FObjectReplicator::ReceivedRPC rva=0x30634b0 rejects a function whose flags do not fit the side.
//!     Whether a server also checks the sending connection against the actor's owner is server code:
//!     engine-UNCONFIRMED (NetServer keeps the check as a guard).
//!   - RepNotify on change: a client calls the OnRep only for a received value that differs from its copy
//!     (RepNotifyCondition 0 = REPNOTIFY_OnChanged in the params): client code (FRepLayout::ReceiveProperties
//!     rva=0x33aaf70, FRepLayout::CallRepNotifies rva=0x33952b0) present but not traced: engine-UNCONFIRMED.

use serde::{Deserialize, Serialize};

use crate::enums::*;
use crate::netmotion::FNetMotion;
use crate::pawn::Pawn;
use crate::player_state::NetPlayerState;
use crate::rep::{NetBlock, NetMotionRep};

pub const CHARACTER_PROPS: &[(&str, u8)] = &[
    (PROP_REPLICATED_NET_MOTION, COND_SKIP_OWNER),
    (PROP_REPLICATED_HEALTH, COND_NONE),
    (PROP_REPLICATED_STAMINA, COND_NONE),
    (PROP_REPLICATED_CHARACTER_FLAGS, COND_NONE),
    (PROP_REPLICATED_LOOK_UP_VALUE, COND_SIMULATED_ONLY),
    (PROP_REPLICATED_TEAM, COND_NONE),
    (PROP_NET_BLOCK, COND_NONE),
    (PROP_REPLICATED_KNOCKBACK, COND_NONE),
    (PROP_REPLICATED_DODGE, COND_SIMULATED_ONLY),
];

/// A property's wire value
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PropValue {
    NetMotion([u8; 6]),
    Byte(u8),
    NetBlock(NetBlock),
}

impl PropValue {
    /// a value still equal to the constructor's (all zero bytes) is what a new client already holds: nothing to send
    pub fn is_initial(&self) -> bool {
        match self {
            PropValue::NetMotion(b) => *b == [0u8; 6],
            PropValue::Byte(b) => *b == 0,
            PropValue::NetBlock(nb) => *nb == NetBlock::default(),
        }
    }
}

/// does a property with this condition go to a connection (owner = the connection owns the pawn)?
pub fn condition_allows(cond: u8, is_owner: bool) -> bool {
    match cond {
        COND_NONE => true,
        COND_SKIP_OWNER => !is_owner,
        COND_SIMULATED_ONLY => !is_owner, // a pawn is simulated on every non-owning client
        _ => false,
    }
}

/// the property's current value on the authority (wire form); `ps` = the owning controller's player state
pub fn value_of(rep: &NetMotionRep, prop: &str, ps: Option<&NetPlayerState>) -> Option<PropValue> {
    Some(match prop {
        PROP_REPLICATED_NET_MOTION => PropValue::NetMotion(rep.replicated.to_bytes()),
        PROP_REPLICATED_HEALTH => PropValue::Byte(rep.replicated_health),
        PROP_REPLICATED_STAMINA => PropValue::Byte(rep.replicated_stamina),
        PROP_REPLICATED_CHARACTER_FLAGS => PropValue::Byte(rep.replicated_character_flags),
        PROP_REPLICATED_LOOK_UP_VALUE => PropValue::Byte(rep.replicated_look_up_value),
        PROP_REPLICATED_TEAM => PropValue::Byte(ps.map(|p| p.replicated_team).unwrap_or(0)),
        PROP_NET_BLOCK => PropValue::NetBlock(rep.net_block.clone()),
        PROP_REPLICATED_KNOCKBACK => PropValue::Byte(rep.replicated_knockback),
        PROP_REPLICATED_DODGE => PropValue::Byte(rep.replicated_dodge),
        _ => return None,
    })
}

/// A client receives a property value: store it; when it differs from the client's copy, run its OnRep
/// (AMordhauCharacter::OnRep_ReplicatedNetMotion rva=0x155ad00, OnRep_ReplicatedHealth rva=0x155abd0,
/// OnRep_ReplicatedStamina rva=0x155aef0, AAdvancedCharacter::OnRep_ReplicatedCharacterFlags rva=0x1491c20,
/// AAdvancedCharacter::OnRep_ReplicatedLookUpValue rva=0x1491cf0, AMordhauPlayerState::OnRep_ReplicatedTeam
/// rva=0x15e6d30, AMordhauCharacter::OnRep_NetBlock rva=0x155a4e0, OnRep_ReplicatedKnockback rva=0x155ac90,
/// OnRep_ReplicatedDodge rva=0x155ab10)
pub fn apply<P: Pawn + ?Sized>(rep: &mut NetMotionRep, p: &mut P, prop: &str, v: &PropValue, ps: Option<&mut NetPlayerState>) {
    let byte = |v: &PropValue| match v {
        PropValue::Byte(b) => Some(*b),
        _ => None,
    };
    match (prop, v) {
        (PROP_REPLICATED_NET_MOTION, PropValue::NetMotion(b)) => {
            if rep.replicated.to_bytes() == *b {
                return;
            }
            rep.replicated = FNetMotion::from_bytes(*b);
            rep.on_rep_replicated_net_motion(p);
        }
        (PROP_NET_BLOCK, PropValue::NetBlock(nb)) => {
            if rep.net_block == *nb {
                return;
            }
            rep.net_block = nb.clone();
            rep.on_rep_net_block(p);
        }
        (PROP_REPLICATED_TEAM, _) => {
            let Some(b) = byte(v) else { return };
            let Some(ps) = ps else { return };
            if ps.replicated_team == b {
                return;
            }
            ps.replicated_team = b;
            ps.on_rep_replicated_team();
            p.set_team(ps.combat_team());
        }
        _ => {
            let Some(b) = byte(v) else { return };
            match prop {
                PROP_REPLICATED_HEALTH => {
                    if rep.replicated_health != b {
                        rep.replicated_health = b;
                        rep.on_rep_replicated_health(p);
                    }
                }
                PROP_REPLICATED_STAMINA => {
                    if rep.replicated_stamina != b {
                        rep.replicated_stamina = b;
                        rep.on_rep_replicated_stamina(p);
                    }
                }
                PROP_REPLICATED_CHARACTER_FLAGS => {
                    if rep.replicated_character_flags != b {
                        rep.replicated_character_flags = b;
                        rep.on_rep_replicated_character_flags(p);
                    }
                }
                PROP_REPLICATED_LOOK_UP_VALUE => {
                    if rep.replicated_look_up_value != b {
                        rep.replicated_look_up_value = b;
                        rep.on_rep_replicated_look_up_value(p);
                    }
                }
                PROP_REPLICATED_KNOCKBACK => {
                    if rep.replicated_knockback != b {
                        rep.replicated_knockback = b;
                        rep.on_rep_replicated_knockback(p);
                    }
                }
                PROP_REPLICATED_DODGE => {
                    if rep.replicated_dodge != b {
                        rep.replicated_dodge = b;
                        rep.on_rep_replicated_dodge(p);
                    }
                }
                _ => {}
            }
        }
    }
}

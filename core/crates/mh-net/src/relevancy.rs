//! Which actors a connection receives, and in what order: relevancy (distance culling) and replication priority,
//! read off the exe.
//!
//! Defaults (AActor::InitializeDefaults rva=0x2e31440, called by every AActor constructor):
//!   NetCullDistanceSquared 2.25e8 = 15000^2 (+0x100, `mov dword [rbx+0x100], 0x4d5693a4` at 0x142e314a8),
//!   NetUpdateFrequency 100 (+0x108), MinNetUpdateFrequency 2 (+0x10c), NetPriority 1 (+0x110).
//! Overrides: APawn::APawn rva=0x32ddce0 NetPriority 3 (0x1432dde09), NetUpdateFrequency 100 (0x1432dde13);
//!   APlayerController::APlayerController 0x332ec70 NetPriority 3; APlayerState::APlayerState 0x3358850
//!   NetUpdateFrequency 1; AInfo::AInfo 0x312abb0 NetUpdateFrequency 10; AMordhauEquipment 0x1526480
//!   NetUpdateFrequency 1. ACharacter / AAdvancedCharacter / AMordhauCharacter / AHorse constructors write none of
//!   +0x100..+0x110, and BP_MordhauCharacter / BP_Horse have no override in their packages (BP_TrebuchetProjectile is
//!   the only Blueprint under Mordhau/Blueprints with a NetCullDistanceSquared).
//!
//! Relevancy of a pawn: AAdvancedCharacter::IsNetRelevantFor 0x1487a70 (SharesInstanceWith, then)
//!   APawn::IsNetRelevantFor rva=0x32ef010: true when bAlwaysRelevant (+0x58 bit 3), the real viewer is the pawn's
//!   Controller (+0x258), the pawn is owned by the view target or the real viewer (Owner chain +0xe0), it is the view
//!   target or the view target's instigator (AActor::GetInstigator 0x2e2cd10), or either is based on the other (vcall
//!   +0x510); false when hidden (+0x58 & 0x24) with no colliding root; a pawn based on a skeletal mesh / its owner
//!   asks its base; else, with AGameNetworkManager bUseDistanceBasedRelevancy (+0x2c1; BaseGame.ini true),
//!   AActor::IsWithinNetRelevancyDistance rva=0x2e32530: (dy^2 + dx^2) + dz^2 < NetCullDistanceSquared.
//!
//! Priority of a pawn: AAdvancedCharacter::GetNetPriority rva=0x1484b60 (ported below, f32 operations in the exe's
//!   order) against the engine's AActor::GetNetPriority rva=0x2e2d710 (other actors).
//!
//! The driver side (UNetDriver::ServerReplicateActors' prioritize / send loop: Time = seconds since the channel's
//! last update, SpawnPrioritySeconds=1.0 for a channel not yet open, actors sent in descending priority until the
//! connection is saturated; a channel closed after RelevantTimeout=5.0 s of irrelevancy; BaseEngine.ini
//! [/Script/OnlineSubsystemUtils.IpNetDriver]) is engine code not disassembled here: its structure is UE 4.26's
//! (UNCONFIRMED), its constants are the config's.

use mordhau_core::ue::FVector;

/// AActor::InitializeDefaults rva=0x2e31440: NetCullDistanceSquared
pub const ACTOR_NET_CULL_DISTANCE_SQUARED: f32 = 225_000_000.0;
/// AActor::InitializeDefaults: NetUpdateFrequency 100, MinNetUpdateFrequency 2, NetPriority 1
pub const ACTOR_NET_UPDATE_FREQUENCY: f32 = 100.0;
pub const ACTOR_MIN_NET_UPDATE_FREQUENCY: f32 = 2.0;
pub const ACTOR_NET_PRIORITY: f32 = 1.0;
/// APawn::APawn rva=0x32ddce0: NetPriority 3
pub const PAWN_NET_PRIORITY: f32 = 3.0;
/// [/Script/OnlineSubsystemUtils.IpNetDriver] RelevantTimeout=5.0 (BaseEngine.ini)
pub const RELEVANT_TIMEOUT: f64 = 5.0;
/// [/Script/OnlineSubsystemUtils.IpNetDriver] SpawnPrioritySeconds=1.0 (BaseEngine.ini)
pub const SPAWN_PRIORITY_SECONDS: f32 = 1.0;
/// MaxInternetClientRate=100000 (BaseEngine.ini / DefaultEngine.ini [/Script/OnlineSubsystemUtils.IpNetDriver]):
/// bytes per second a server sends one client at most (ConfiguredInternetSpeed 6400000 is clamped to it)
pub const MAX_INTERNET_CLIENT_RATE: f32 = 100_000.0;
/// AGameNetworkManager bUseDistanceBasedRelevancy=true (BaseGame.ini)
pub const USE_DISTANCE_BASED_RELEVANCY: bool = true;

/// AActor::IsWithinNetRelevancyDistance rva=0x2e32530 (`setb`: strictly closer)
pub fn is_within_net_relevancy_distance(actor: FVector, src: FVector, cull_sq: f32) -> bool {
    let dy = actor.y - src.y;
    let dx = actor.x - src.x;
    let dz = actor.z - src.z;
    (dy * dy + dx * dx) + dz * dz < cull_sq
}

/// what APawn::IsNetRelevantFor reads, for one (pawn, viewer)
#[derive(Clone, Copy, Debug)]
pub struct PawnRelevancyQuery {
    /// bAlwaysRelevant (+0x58 bit 3)
    pub always_relevant: bool,
    /// the real viewer (player controller) is the pawn's Controller
    pub viewer_is_controller: bool,
    /// owned by the view target or the real viewer, is the view target, the view target's instigator, or based on
    /// / a base of the view target
    pub tied_to_viewer: bool,
    /// hidden (+0x58 & 0x24) with no colliding root
    pub hidden_without_collision: bool,
    /// a base that decides for the pawn (skeletal mesh base / the owner's base): its own relevancy
    pub base_relevant: Option<bool>,
    pub location: FVector,
    pub net_cull_distance_squared: f32,
}

/// APawn::IsNetRelevantFor rva=0x32ef010 (module docs); `src` = the viewer's view location
pub fn pawn_is_net_relevant_for(q: &PawnRelevancyQuery, src: FVector) -> bool {
    if q.always_relevant || q.viewer_is_controller || q.tied_to_viewer {
        return true;
    }
    if q.hidden_without_collision {
        return false;
    }
    if let Some(b) = q.base_relevant {
        return b;
    }
    !USE_DISTANCE_BASED_RELEVANCY || is_within_net_relevancy_distance(q.location, src, q.net_cull_distance_squared)
}

/// AAdvancedCharacter::IsNetRelevantFor 0x1487a70 (src/Mordhau/Private/AdvancedCharacter.cpp): the viewer's
/// AMordhauPlayerController SharesInstanceWith the pawn, then APawn's
pub fn advanced_character_is_net_relevant_for(shares_instance: bool, q: &PawnRelevancyQuery, src: FVector) -> bool {
    shares_instance && pawn_is_net_relevant_for(q, src)
}

/// AAdvancedCharacter::GetNetPriority rva=0x1484b60. `view_pos` / `view_dir` the viewer's view point and direction,
/// `viewer_target` = this pawn is the view target or the view target is its instigator (0x141484bb2..0x141484bce),
/// `location` = None when hidden (+0x58 bit 5) or without a root component, `time` = seconds since this actor's last
/// replication to the connection, `net_priority` = NetPriority (+0x110).
///   view target -> Time * 4; else Dir = Location - ViewPos, DistSq = |Dir|^2, Dot = Dir . ViewDir, in the 45 degree
///   cone = Dot^2 > DistSq * 0.5:
///   behind (Dot < 0): DistSq > 1500^2 -> x0.2, > 500^2 -> x0.4, else x1;
///   in front: DistSq < 500^2 -> x2; < 1500^2 -> x1; < 3000^2 -> x1 in the cone, else x0.4;
///   farther: < 6000^2 in the cone -> x0.4, else x0.2 (.rdata 0x1443247b8.. 250000 / 2.25e6 / 9e6 / 3.6e7)
///   then * NetPriority. (AActor's 0x2e2d710 differs: behind 2000^2 / 500^2; in front within 8000^2 in the cone x2,
///   else beyond 9998244 x0.4; and its
///   view target test reads Instigator directly.)
pub fn advanced_character_get_net_priority(view_pos: FVector, view_dir: FVector, location: Option<FVector>, viewer_target: bool, time: f32, net_priority: f32) -> f32 {
    let f = if viewer_target {
        time * 4.0
    } else if let Some(l) = location {
        let (dx, dy, dz) = (l.x - view_pos.x, l.y - view_pos.y, l.z - view_pos.z);
        let dist_sq = (dx * dx + dy * dy) + dz * dz;
        let dot = (dy * view_dir.y + dx * view_dir.x) + dz * view_dir.z;
        let cone = dot * dot > dist_sq * 0.5;
        if dot < 0.0 {
            if dist_sq > 2_250_000.0 {
                time * 0.2
            } else if dist_sq > 250_000.0 {
                time * 0.4
            } else {
                time
            }
        } else if dist_sq < 250_000.0 {
            time + time
        } else if dist_sq < 2_250_000.0 {
            time
        } else if dist_sq < 9_000_000.0 {
            if cone {
                time
            } else {
                time * 0.4
            }
        } else if dist_sq >= 36_000_000.0 || !cone {
            time * 0.2
        } else {
            time * 0.4
        }
    } else {
        time
    };
    f * net_priority
}

/// AActor::GetNetPriority rva=0x2e2d710 (actors other than pawns; `viewer_target` = this is the view target or its
/// Instigator (+0x118) is)
pub fn actor_get_net_priority(view_pos: FVector, view_dir: FVector, location: Option<FVector>, viewer_target: bool, time: f32, net_priority: f32) -> f32 {
    let f = if viewer_target {
        time * 4.0
    } else if let Some(l) = location {
        let (dx, dy, dz) = (l.x - view_pos.x, l.y - view_pos.y, l.z - view_pos.z);
        let dist_sq = (dy * dy + dx * dx) + dz * dz;
        let dot = (dy * view_dir.y + dx * view_dir.x) + dz * view_dir.z;
        if dot < 0.0 {
            if dist_sq > 4_000_000.0 {
                time * 0.2
            } else if dist_sq > 250_000.0 {
                time * 0.4
            } else {
                time
            }
        } else if dist_sq < 64_000_000.0 && dot * dot > dist_sq * 0.5 {
            // .rdata 0x144948bcc = 6.4e7 (8000^2)
            time + time
        } else if dist_sq > f32::from_bits(0x4b18_8fa4) {
            // .rdata 0x144948bc8 = 9998244 (~3162^2), its flags feeding the 0x142e2d809 `jbe`
            time * 0.4
        } else {
            time
        }
    } else {
        time
    };
    f * net_priority
}

/// UNetDriver's per-connection send loop (UNCONFIRMED structure, see the module docs): priority = RoundToInt(65536 x
/// GetNetPriority) per relevant actor, sorted high first; actors are sent while the frame's byte budget
/// (MaxInternetClientRate x DeltaTime) lasts. Returns the indices sent, in order. `cost` = an actor's bytes.
pub fn send_order(priorities: &[f32], cost: &[u32], budget_bytes: f32) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..priorities.len()).collect();
    let p: Vec<i32> = priorities.iter().map(|&x| crate::quant::round_to_int(65536.0 * x)).collect();
    idx.sort_by(|&a, &b| p[b].cmp(&p[a]).then(a.cmp(&b)));
    let mut used = 0.0f32;
    let mut out = vec![];
    for i in idx {
        if used >= budget_bytes {
            break;
        }
        used += cost[i] as f32;
        out.push(i);
    }
    out
}

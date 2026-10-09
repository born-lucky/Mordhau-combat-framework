//! The messages the rules hand to a transport: the shipped RPCs and replicated properties by name, plus the actor
//! channel / beacon connection stand-ins. Engine-agnostic data; every transport carries the encoded bytes
//! (`encode` / `decode`), so what the loopback delivers is exactly what a socket carries (the GDScript port does the
//! same with var_to_bytes).

use serde::{Deserialize, Serialize};

use crate::enums::*;
use crate::layout::PropValue;

/// FPlayFabEntity / FPlayFabPlayerEntity (ID string + Type), as the beacon carries it
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Entity {
    pub id: String,
    pub r#type: i32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Msg {
    // ---- client -> server RPCs of a pawn (AMordhauCharacter) ----
    ServerAssignNetMotion { who: String, nm: [u8; 6], last: u8 },
    ServerDropParry { who: String, id: u8 },
    ServerSuggestHitDetection { who: String, other: String, bone: String },
    ServerRequestDodge { who: String, yaw: u8 },
    // ---- server -> owning client RPC ----
    ClientSetNetMotion { who: String, nm: [u8; 6], t: f64 },
    // ---- property replication ----
    Prop { who: String, prop: String, v: PropValue },
    /// a property of the client's own PlayerController (e.g. BP_DuelPlayerController ReplicatedRoomGame)
    PcProp { prop: String, v: serde_json::Value },
    /// actor channel open: a replicated pawn spawns on the client (engine-UNCONFIRMED mechanics, see server.rs)
    Spawn { name: String, owner: u32, weapon: String, left: String, team: u8 },
    /// actor channel closed
    Destroy { name: String },
    // ---- beacon (AMordhauBeaconClient) ----
    BeaconConnect { id: u32 },
    BeaconAccept,
    BeaconRefused,
    ServerPing,
    ClientPong,
    ServerReserveSlots { entities: Vec<Entity> },
    ClientNotifyReservationStatus { open: i32, status: u8 },
    // ---- character movement (movement.rs) ----
    /// ServerMove / ServerMoveDual / ServerMoveOld (unreliable in the engine: a stale one is dropped by its timestamp)
    /// gen: the pawn's spawn number (a new actor each spawn in the engine: moves for an older one are dropped)
    ServerMove { who: String, gen: u32, data: crate::movement::ServerMovePacket },
    /// AActor ReplicatedMovement + ACharacter ReplicatedMovementMode to simulated proxies
    RepMovement { who: String, r: crate::movement::RepMovement },
    /// a pawn's spawn state (the actor channel's initial replication: location and rotation), to every client;
    /// kind: the pawn class (0 character, 1 horse)
    MovementSpawn { who: String, gen: u32, r: crate::movement::RepMovement, #[serde(default)] kind: u8 },
    /// ClientAckGoodMove / ClientAdjustPosition to the owning client
    MoveResponse { who: String, r: crate::movement::MoveResponse },
    /// BP_MordhauCharacter ServerSetClimbLocation(Vector_NetQuantize) (reliable server RPC; package
    /// Mordhau/Content/Mordhau/Blueprints/Characters/BP_MordhauCharacter): ClimbTargetLocation on the server
    ServerSetClimbLocation { who: String, gen: u32, target: [f32; 3] },
    /// AHorse::ServerRequestRearing (reliable server RPC; _Implementation rva=0x15155b0)
    ServerRequestRearing { who: String, gen: u32 },
    /// a movement pawn's actor channel closed for this connection (no longer relevant for RelevantTimeout, or gone):
    /// the proxy is destroyed (relevancy.rs)
    MovementDestroy { who: String },
    /// the receiving client now drives (true) / no longer drives (false) this movement pawn (a horse it mounted:
    /// UMordhauVehicleComponent::StartDriving rva=0x14d79a0 possesses it; StopDriving rva=0x14d8270 unpossesses)
    MovementPossess { who: String, gen: u32, owned: bool },
    /// AActor AttachmentReplication (FRepAttachment: AttachParent, LocationOffset FVector_NetQuantize100, RotationOffset
    /// FRotator - FRotator::NetSerialize rva=0x18b55d0 = SerializeCompressedShort): a rider attached to its horse's mesh
    /// (UMordhauVehicleComponent::StartDriving rva=0x14d79a0 AttachToComponent); parent None = detached
    RepAttachment { who: String, parent: Option<String>, offset: [f32; 3], yaw: u16 },
    /// a replicated property of a placed gameplay actor (world_rep.rs), server to client
    WorldProp { actor: u32, var: String, value: i64 },
    // ---- process-to-process session control (not game RPCs; the stand-in for UE's NMT_Login / NMT_Welcome
    // handshake and for the lockstep frame of NetSession when server and clients run in separate processes) ----
    /// a client process asks to log in with its URL options
    Join { name: String, options: String },
    /// the server's answer: "" = approved (with the PlayerId and the server clock), else the error (NMT_Failure)
    JoinResult { error: String, player_id: i32, tick_n: u64, now: f64 },
    /// a client finished its frame n (all its RPCs of frame n were sent before this)
    FrameDone { n: u64 },
    /// the server finished frame n (all its messages of frame n were sent before this); quit = the session ends
    FrameEnd { n: u64, quit: bool },
}

impl Msg {
    /// a session-control message (Join / JoinResult / FrameDone / FrameEnd): handled by the process loop, not the rules
    /// a character-movement message (movement.rs): handled by the node, not the combat rules
    pub fn is_movement(&self) -> bool {
        matches!(
            self,
            Msg::ServerMove { .. }
                | Msg::RepMovement { .. }
                | Msg::MovementSpawn { .. }
                | Msg::MoveResponse { .. }
                | Msg::ServerSetClimbLocation { .. }
                | Msg::ServerRequestRearing { .. }
                | Msg::MovementDestroy { .. }
                | Msg::MovementPossess { .. }
                | Msg::RepAttachment { .. }
                | Msg::WorldProp { .. }
        )
    }

    pub fn is_control(&self) -> bool {
        matches!(self, Msg::Join { .. } | Msg::JoinResult { .. } | Msg::FrameDone { .. } | Msg::FrameEnd { .. })
    }
}

impl Msg {
    /// the shipped RPC / property name this message carries ("" for channel / connection messages)
    pub fn name(&self) -> &str {
        match self {
            Msg::ServerAssignNetMotion { .. } => RPC_SERVER_ASSIGN_NET_MOTION,
            Msg::ServerDropParry { .. } => RPC_SERVER_DROP_PARRY,
            Msg::ServerSuggestHitDetection { .. } => RPC_SERVER_SUGGEST_HIT_DETECTION,
            Msg::ServerRequestDodge { .. } => RPC_SERVER_REQUEST_DODGE,
            Msg::ClientSetNetMotion { .. } => RPC_CLIENT_SET_NET_MOTION,
            Msg::Prop { prop, .. } => prop,
            Msg::PcProp { prop, .. } => prop,
            Msg::ServerPing => RPC_SERVER_PING,
            Msg::ClientPong => RPC_CLIENT_PONG,
            Msg::ServerReserveSlots { .. } => RPC_SERVER_RESERVE_SLOTS,
            Msg::ClientNotifyReservationStatus { .. } => RPC_CLIENT_NOTIFY_RESERVATION_STATUS,
            Msg::ServerMove { .. } => "ServerMove",
            Msg::RepMovement { .. } => "ReplicatedMovement",
            Msg::MoveResponse { r: crate::movement::MoveResponse::AckGoodMove { .. }, .. } => "ClientAckGoodMove",
            Msg::MoveResponse { .. } => "ClientAdjustPosition",
            Msg::ServerSetClimbLocation { .. } => "ServerSetClimbLocation",
            Msg::ServerRequestRearing { .. } => "ServerRequestRearing",
            _ => "",
        }
    }

    /// the pawn a pawn RPC / property is about ("" otherwise)
    pub fn who(&self) -> &str {
        match self {
            Msg::ServerAssignNetMotion { who, .. }
            | Msg::ServerDropParry { who, .. }
            | Msg::ServerSuggestHitDetection { who, .. }
            | Msg::ServerRequestDodge { who, .. }
            | Msg::ClientSetNetMotion { who, .. }
            | Msg::ServerMove { who, .. }
            | Msg::RepMovement { who, .. }
            | Msg::MovementSpawn { who, .. }
            | Msg::MoveResponse { who, .. }
            | Msg::ServerSetClimbLocation { who, .. }
            | Msg::ServerRequestRearing { who, .. }
            | Msg::MovementDestroy { who }
            | Msg::MovementPossess { who, .. }
            | Msg::RepAttachment { who, .. }
            | Msg::Prop { who, .. } => who,
            _ => "",
        }
    }

    /// the message on its own in the engine's bit forms (wire.rs); a connection (transport.rs) frames it in bunches
    pub fn encode(&self) -> Vec<u8> {
        crate::wire::encode_standalone(self)
    }

    pub fn decode(b: &[u8]) -> Option<Msg> {
        crate::wire::decode_standalone(b)
    }
}

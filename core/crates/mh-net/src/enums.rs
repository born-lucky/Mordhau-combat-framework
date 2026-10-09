//! The engine enum values the replication rules compare against, each with the exe evidence for it
//! (port of godot/game/net/net_enums.gd).
//!
//!   ENetRole: UHT name table in the exe .rdata, in value order: ROLE_None 0x144b1f260, ROLE_SimulatedProxy
//!     0x144b1f270, ROLE_AutonomousProxy 0x144b1f288, ROLE_Authority 0x144b1f2a0. The game code tests 3 (AActor Role
//!     +0xf0, `UMotionSystemComponent::AssignNetMotion` at 0x1414b3531: cmp byte [rbx+0xf0], 3) and 2
//!     (`UMotionSystemComponent::OnRep_ReplicatedNetMotion` at 0x1414ce818: cmp eax, 2 after
//!     GetControllerRoleIncludingVehicle).
//!   ELifetimeCondition: name table in the exe .rdata, in value order: COND_None 0x1444f74b0, COND_InitialOnly
//!     0x1444f74c0, COND_OwnerOnly 0x1444f74d8, COND_SkipOwner 0x1444f74e8, COND_SimulatedOnly 0x1444f74f8,
//!     COND_AutonomousOnly 0x1444f7510. AMordhauCharacter::GetLifetimeReplicatedProps rva=0x153fd60 writes the
//!     condition as the first byte of FDoRepLifetimeParams before each RegisterReplicatedLifetimeProperty.
//!   Net mode 3: `UMotionSystemComponent::OnClientSetNetMotion` at 0x1414ca20c (cmp eax, 3 on
//!     AActor::InternalGetNetMode) only applies ClientSetNetMotion on a client. UNCONFIRMED name: no ENetMode name table
//!     was found in the exe; 3 is taken as "client" because only the owning client is sent that RPC.

pub const ROLE_SIMULATED_PROXY: u8 = 1;
pub const ROLE_AUTONOMOUS_PROXY: u8 = 2;
pub const ROLE_AUTHORITY: u8 = 3;

pub const COND_NONE: u8 = 0;
pub const COND_SKIP_OWNER: u8 = 3;
pub const COND_SIMULATED_ONLY: u8 = 4;

pub const NET_MODE_CLIENT: u8 = 3;
/// UNCONFIRMED value: any mode other than 3 makes OnClientSetNetMotion a no-op.
/// AGameSession::AtCapacity rva=0x30cb0c0 tests net mode 0 (`test eax, eax`: returns false) and 2 (`cmp eax, 2`: the
/// spectator rule's listen-server case); AGameSession::UnregisterPlayer rva=0x30f0e60 also skips mode 0. UNCONFIRMED
/// names (no ENetMode name table in the exe): 0 standalone, 1 dedicated server, 2 listen server, 3 client.
pub const NET_MODE_STANDALONE: u8 = 0;
pub const NET_MODE_DEDICATED_SERVER: u8 = 1;
pub const NET_MODE_LISTEN_SERVER: u8 = 2;
pub const NET_MODE_SERVER: u8 = NET_MODE_DEDICATED_SERVER;

// RPCs (UFunction FuncParams in .data: FunctionFlags at +0x40). The flag bits are UE 4.26 EFunctionFlags; their names
// are not in the exe (UNCONFIRMED names): 0x40 Net, 0x80 NetReliable, 0x200000 NetServer, 0x1000000 NetClient,
// 0x80000000 NetValidate.
/// ServerAssignNetMotion(FNetMotion NewNetMotion, uint8 LastAuthObserved): FuncParams 0x1454a7bb0, flags 0x80220cc0
/// (reliable, server, validated; AMordhauCharacter::ServerAssignNetMotion_Validate rva=0x7bf3e0 returns true)
pub const RPC_SERVER_ASSIGN_NET_MOTION: &str = "ServerAssignNetMotion";
/// ServerDropParry(uint8 MotionID): FuncParams 0x1454a7c00, flags 0x80220cc0 (reliable, server, validated)
pub const RPC_SERVER_DROP_PARRY: &str = "ServerDropParry";
/// ClientSetNetMotion(FNetMotion NewMotion, float ServerStartTime): FuncParams 0x1454a5040, flags 0x1020cc0
/// (reliable, client)
pub const RPC_CLIENT_SET_NET_MOTION: &str = "ClientSetNetMotion";
/// ServerSuggestHitDetection(AAdvancedCharacter* OtherCharacter, FVector_NetQuantize HitLocation, uint8 BoneId):
/// FuncParams 0x1454a7e30, flags 0x80220cc0 (reliable, server, validated; _Validate rva=0x7bf3e0 returns true)
pub const RPC_SERVER_SUGGEST_HIT_DETECTION: &str = "ServerSuggestHitDetection";
/// ServerRequestDodge(uint8 PackedWorldYaw): FuncParams 0x1454a7cf0, flags 0x84220cc0 (the server-RPC bits above
/// plus 0x4000000, UNCONFIRMED name; _Validate rva=0x7bf3e0 returns true)
pub const RPC_SERVER_REQUEST_DODGE: &str = "ServerRequestDodge";

// Replicated properties of a character the combat state needs (see layout.rs)
pub const PROP_REPLICATED_NET_MOTION: &str = "ReplicatedNetMotion";
pub const PROP_REPLICATED_HEALTH: &str = "ReplicatedHealth";
pub const PROP_REPLICATED_STAMINA: &str = "ReplicatedStamina";
pub const PROP_REPLICATED_CHARACTER_FLAGS: &str = "ReplicatedCharacterFlags";
pub const PROP_REPLICATED_LOOK_UP_VALUE: &str = "ReplicatedLookUpValue";
pub const PROP_NET_BLOCK: &str = "NetBlock";
pub const PROP_REPLICATED_KNOCKBACK: &str = "ReplicatedKnockback";
pub const PROP_REPLICATED_DODGE: &str = "ReplicatedDodge";
/// AMordhauPlayerState (the pawn owner's player state)
pub const PROP_REPLICATED_TEAM: &str = "ReplicatedTeam";

// Beacon RPCs (AMordhauBeaconClient, a separate beacon connection before the game connection)
pub const RPC_SERVER_PING: &str = "ServerPing";
pub const RPC_CLIENT_PONG: &str = "ClientPong";
pub const RPC_SERVER_RESERVE_SLOTS: &str = "ServerReserveSlots";
pub const RPC_CLIENT_NOTIFY_RESERVATION_STATUS: &str = "ClientNotifyReservationStatus";
// EReservationStatus (uint8, PDB enum, extract/native/types/EReservationStatus.h): Success 0, Full 1, Failure 2
pub const RESERVATION_SUCCESS: u8 = 0;
pub const RESERVATION_FULL: u8 = 1;
pub const RESERVATION_FAILURE: u8 = 2;
// EBeaconRequest (uint8, PDB enum; member names not read, UNCONFIRMED): AMordhauBeaconClient::Ping rva=0x15128b0
// stores 0 in Request (+0x2b0), AMordhauBeaconClient::ReserveSlots rva=0x1514ec0 stores 1
pub const BEACON_REQUEST_PING: i32 = 0;
pub const BEACON_REQUEST_RESERVE_SLOTS: i32 = 1;

/// The combat enum values the net rules read. They mirror godot/game/combat/combat_enums.gd (EAttackMove LF_ENUM of
/// the shipped PDB; EAttackStage from the UHT name table "EAttackStage::Windup" @file 0x43882c0; FNetMotion
/// MotionType literals of FNetMotion::Flinched rva=0x14bb820 / ::Blocked rva=0x1615f50 / ::Stunned rva=0x14d86a0 /
/// ::Disarmed rva=0x14ba4a0; EBlockedReason types/EBlockedReason.h). Owned by mordhau-core (rust-combat): switch to
/// its enums once they land.
pub mod combat {
    pub const MOVE_RIGHT_STRIKE: u8 = 0;
    pub const MOVE_LEFT_STRIKE: u8 = 1;
    pub const MOVE_STAB: u8 = 2;
    pub const MOVE_ALT_STAB: u8 = 3;
    pub const MOVE_KICK: u8 = 4;
    pub const MOVE_RANGED: u8 = 7;
    pub const NET_ATTACK: u8 = 1;
    pub const NET_PARRY: u8 = 2;
    pub const NET_FLINCHED: u8 = 3;
    pub const NET_STUNNED: u8 = 4;
    pub const NET_FEINTED: u8 = 5;
    pub const NET_BLOCKED: u8 = 6;
    pub const NET_DISARMED: u8 = 7;
    pub const BLOCKED_PARRY: u8 = 0;
    pub const BLOCKED_CHAMBER: u8 = 1;
    pub const BLOCKED_WORLD: u8 = 2;
    pub const BLOCKED_CLASH: u8 = 3;
    pub const BLOCKED_HIT: u8 = 4;
}

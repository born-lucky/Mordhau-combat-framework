//! The .rdata literals of the replication rules (port of godot/components/ue/rdata/net_constants.gd). The GDScript
//! port reads each through UeRdata at load; an engine-neutral core crate does no I/O, so the raw bits are written
//! here next to their virtual address, and `CITES` lists every one so a test (and verify_constants-style tooling)
//! can check the bits against extract/native/rdata.tsv. Values below are the `raw` column of rdata.tsv.

/// name, .rdata va, raw f32 bits, game functions whose code loads it (rdata.tsv funcs column)
pub const CITES: &[(&str, u64, u32, &[&str])] = &[
    ("stat_byte_to_unit", 0x144014a94, 0x3c23d70a, &["UStatComponent::ReadReplicatedStat"]),
    ("stat_unit_to_byte_x2", 0x14432478c, 0x43480000, &["UStatComponent::WriteReplicatedStat"]),
    ("stat_round_half", 0x143fe4e04, 0x3f000000, &["UStatComponent::ReadReplicatedStat", "UStatComponent::WriteReplicatedStat"]),
    ("stat_unit_max", 0x143fe4e0c, 0x3f800000, &["UStatComponent::ReadReplicatedStat", "UStatComponent::WriteReplicatedStat"]),
    ("stat_small_number", 0x144014a88, 0x322bcc77, &["UStatComponent::WriteReplicatedStat"]),
    ("ping_ms_to_s", 0x144014a8c, 0x3a83126f, &["UMordhauUtilityLibrary::GetPing", "UAttackMotion::OnBegin_Implementation"]),
    ("ping_sample_max", 0x143fe4e0c, 0x3f800000, &["AMordhauPlayerState::UpdatePing"]),
    ("ping_median_half", 0x143fe4e04, 0x3f000000, &["AMordhauPlayerState::UpdatePing"]),
    ("ping_median_round", 0x144022404, 0xbf000000, &["AMordhauPlayerState::UpdatePing"]),
    ("lag_induction_max", 0x1442efa58, 0x3e19999a, &["UAttackMotion::OnBegin_Implementation"]),
    ("lag_induction_max_netcode2", 0x14402c9a0, 0x3ccccccd, &["UAttackMotion::OnBegin_Implementation"]),
    ("lag_induction_stab_factor", 0x143fe4e04, 0x3f000000, &["UAttackMotion::OnBegin_Implementation"]),
    ("lag_unit", 0x143fe4e0c, 0x3f800000, &["UAttackMotion::OnBegin_Implementation"]),
    ("ping_extrapolation_factor", 0x143fe4e04, 0x3f000000, &["`dynamic initializer for 'CVarPingExtrapolationFactor''"]),
    ("look_up_to_byte", 0x144014b78, 0x437f0000, &["AAdvancedCharacter::PreReplication"]),
    ("look_up_byte_to_unit", 0x1440a4394, 0x3b808081, &["AAdvancedCharacter::OnRep_ReplicatedLookUpValue"]),
    ("look_small_number", 0x144014a88, 0x322bcc77, &["AAdvancedCharacter::PreReplication"]),
    ("look_unit_max", 0x143fe4e0c, 0x3f800000, &["AAdvancedCharacter::PreReplication", "AAdvancedCharacter::OnRep_ReplicatedLookUpValue"]),
    ("net_block_min_world_time", 0x1440701e8, 0x40e00000, &["AMordhauCharacter::OnRep_NetBlock"]),
    ("knockback_min_size", 0x143fe4dfc, 0x3dcccccd, &["AMordhauCharacter::Knockback"]),
    ("dodge_byte_to_deg", 0x144349d98, 0x3fb4b4b5, &["AMordhauCharacter::OnRep_ReplicatedDodge", "AMordhauCharacter::ServerRequestDodge_Implementation"]),
    ("dodge_unwind_max", 0x14402cba4, 0x43340000, &["AMordhauCharacter::OnRep_ReplicatedDodge", "AMordhauCharacter::ServerRequestDodge_Implementation"]),
    ("dodge_unwind_turn", 0x1442713d4, 0x43b40000, &["AMordhauCharacter::OnRep_ReplicatedDodge", "AMordhauCharacter::ServerRequestDodge_Implementation", "UMordhauMovementComponent::LODTick"]),
    ("dodge_unwind_min", 0x1442713e4, 0xc3340000, &["AMordhauCharacter::OnRep_ReplicatedDodge", "AMordhauCharacter::ServerRequestDodge_Implementation"]),
    ("dodge_unwind_neg_turn", 0x1442713e8, 0xc3b40000, &["AMordhauCharacter::OnRep_ReplicatedDodge", "AMordhauCharacter::ServerRequestDodge_Implementation"]),
    ("dodge_rad_to_deg", 0x1442713d0, 0x42652ee0, &["UMordhauMovementComponent::LODTick"]),
    ("dodge_deg_to_byte", 0x144331158, 0x3f355556, &["UMordhauMovementComponent::LODTick"]),
    ("dodge_round_half", 0x143fe4e04, 0x3f000000, &["UMordhauMovementComponent::LODTick"]),
    ("ban_seconds_to_minutes", 0x14428988c, 0x3c888889, &["AMordhauGameSession::GetPlayerBanDuration"]),
    ("ban_round_neg_half", 0x144022404, 0xbf000000, &["AMordhauGameSession::GetPlayerBanDuration"]),
    ("pong_round_half", 0x143fe4e04, 0x3f000000, &["AMordhauBeaconClient::ClientPong_Implementation"]),
];

/// f64 literals: name, .rdata va, raw bits, functions
pub const CITES64: &[(&str, u64, u64, &[&str])] = &[
    // 1000: seconds -> milliseconds of a beacon ping, a double (mulsd at 0x1414f4bd6)
    ("pong_s_to_ms", 0x143fe4e18, 0x408f400000000000, &["AMordhauBeaconClient::ClientPong_Implementation"]),
];

const fn f(bits: u32) -> f32 {
    f32::from_bits(bits)
}

// ---- replicated stat byte -----------------------------------------------------------------------------------------
/// 0.01: a replicated stat byte (percent, low 7 bits) back to a fraction: (Byte & 0x7f) x 0.01 (va 0x144014a94)
pub const STAT_BYTE_TO_UNIT: f32 = f(0x3c23d70a);
/// 200: fraction x 200 + 0.5, cvtss2si, >> 1 = RoundToInt(fraction x 100) (va 0x14432478c)
pub const STAT_UNIT_TO_BYTE_X2: f32 = f(0x43480000);
/// 0.5: the + 0.5 of both round-to-int sequences (va 0x143fe4e04)
pub const STAT_ROUND_HALF: f32 = f(0x3f000000);
/// 1: the clamp of the fraction to [0, 1] (va 0x143fe4e0c)
pub const STAT_UNIT_MAX: f32 = f(0x3f800000);
/// 1e-8: |Max - Min| <= it is a degenerate range (va 0x144014a88)
pub const STAT_SMALL_NUMBER: f32 = f(0x322bcc77);

// ---- ping compensation --------------------------------------------------------------------------------------------
/// 0.001: ExactPing (ms) -> s (va 0x144014a8c, UMordhauUtilityLibrary::GetPing / UAttackMotion::OnBegin_Implementation)
pub const PING_MS_TO_S: f32 = f(0x3a83126f);
/// 1: UpdatePing clamps the new sample to <= 1 s (minss at 0x141604583; va 0x143fe4e0c)
pub const PING_SAMPLE_MAX: f32 = f(0x3f800000);
/// 0.5 and -0.5: the median index CeilToInt(Num x 0.5) = -(cvtss2si(-0.5 - (Num x 0.5) x 2) >> 1)
/// (0x141604600..0x14160462e; va 0x143fe4e04 / 0x144022404)
pub const PING_MEDIAN_HALF: f32 = f(0x3f000000);
pub const PING_MEDIAN_ROUND: f32 = f(0xbf000000);
/// 0.15: LagInduction cap on the owning client (minss at 0x14162f534, NetcodeType 0; va 0x1442efa58)
pub const LAG_INDUCTION_MAX: f32 = f(0x3e19999a);
/// 0.025: LagInduction cap with NetcodeType 2 (minss at 0x14162f515; va 0x14402c9a0)
pub const LAG_INDUCTION_MAX_NETCODE2: f32 = f(0x3ccccccd);
/// 0.5: stabs take half the LagInduction; strikes take (1 - |AngleTarget| x 0.5) of it (va 0x143fe4e04)
pub const LAG_INDUCTION_STAB_FACTOR: f32 = f(0x3f000000);
/// 1: the 1 - |AngleTarget| x 0.5 term and the [0, 1] clamp of PingExtrapolationFactor (va 0x143fe4e0c)
pub const LAG_UNIT: f32 = f(0x3f800000);
/// 0.5: m.PingExtrapolationFactor CVar default (its initializer loads it into xmm2; va 0x143fe4e04)
pub const PING_EXTRAPOLATION_FACTOR: f32 = f(0x3f000000);

// ---- ReplicatedLookUpValue ----------------------------------------------------------------------------------------
/// 255: fraction x 255 -> byte (AAdvancedCharacter::PreReplication mulss at 0x141499dc0; va 0x144014b78)
pub const LOOK_UP_TO_BYTE: f32 = f(0x437f0000);
/// 1/255: byte -> fraction (AAdvancedCharacter::OnRep_ReplicatedLookUpValue mulss at 0x141491e1f; va 0x1440a4394)
pub const LOOK_UP_BYTE_TO_UNIT: f32 = f(0x3b808081);
/// 1e-8: a degenerate look range (va 0x144014a88)
pub const LOOK_SMALL_NUMBER: f32 = f(0x322bcc77);
/// 1: the [0, 1] clamp of the fraction (va 0x143fe4e0c)
pub const LOOK_UNIT_MAX: f32 = f(0x3f800000);

// ---- NetBlock / ReplicatedKnockback / ReplicatedDodge ---------------------------------------------------------------
/// 7: OnRep_NetBlock returns while World TimeSeconds <= 7 (va 0x1440701e8, AMordhauCharacter::OnRep_NetBlock)
pub const NET_BLOCK_MIN_WORLD_TIME: f32 = f(0x40e00000);
/// 0.1: Knockback does nothing for |Amount| <= 0.1 (va 0x143fe4dfc, AMordhauCharacter::Knockback)
pub const KNOCKBACK_MIN_SIZE: f32 = f(0x3dcccccd);
/// 360/255: ReplicatedDodge byte -> world yaw degrees (va 0x144349d98)
pub const DODGE_BYTE_TO_DEG: f32 = f(0x3fb4b4b5);
/// 180 / 360 / -180 / -360: the unwind loops of the dodge yaw (va 0x14402cba4, 0x1442713d4, 0x1442713e4, 0x1442713e8)
pub const DODGE_UNWIND_MAX: f32 = f(0x43340000);
pub const DODGE_UNWIND_TURN: f32 = f(0x43b40000);
pub const DODGE_UNWIND_MIN: f32 = f(0xc3340000);
pub const DODGE_UNWIND_NEG_TURN: f32 = f(0xc3b40000);
/// 57.295776: Atan2 radians -> degrees of the dodge direction (va 0x1442713d0, UMordhauMovementComponent::LODTick)
pub const DODGE_RAD_TO_DEG: f32 = f(0x42652ee0);
/// 255/360: yaw degrees -> PackedWorldYaw (va 0x144331158, UMordhauMovementComponent::LODTick)
pub const DODGE_DEG_TO_BYTE: f32 = f(0x3f355556);
/// 0.5 (va 0x143fe4e04, UMordhauMovementComponent::LODTick)
pub const DODGE_ROUND_HALF: f32 = f(0x3f000000);

// ---- game session / beacon ----------------------------------------------------------------------------------------
/// 1/60: seconds -> minutes of a ban (AMordhauGameSession::GetPlayerBanDuration mulss at 0x141597f32; va 0x14428988c)
pub const BAN_SECONDS_TO_MINUTES: f32 = f(0x3c888889);
/// -0.5: the CeilToInt of the minutes, -(cvtss2si(-0.5 - 2x) >> 1) (va 0x144022404)
pub const BAN_ROUND_NEG_HALF: f32 = f(0xbf000000);
/// 0.5: RoundToInt of the pong milliseconds (`AMordhauBeaconClient::ClientPong_Implementation` addss at 0x1414f4be6;
/// va 0x143fe4e04)
pub const PONG_ROUND_HALF: f32 = f(0x3f000000);
/// 1000.0 (f64): seconds -> milliseconds (mulsd at 0x1414f4bd6; va 0x143fe4e18)
pub const PONG_S_TO_MS: f64 = f64::from_bits(0x408f400000000000);

// ---- [/Script/Engine.GameSession] of extract/config/DefaultGame.ini (AGameSession config) ---------------------------
// Package data, not code: the GDScript port reads it through UeConfig; this crate has no I/O, so the shipped values
// are the defaults of `SessionConfig` and a host can pass the ini's values instead (mh-spec).
/// MaxPlayers=16 (DefaultGame.ini [/Script/Engine.GameSession])
pub const SESSION_MAX_PLAYERS: i32 = 16;
/// MaxSpectators=0 (DefaultGame.ini [/Script/Engine.GameSession])
pub const SESSION_MAX_SPECTATORS: i32 = 0;
/// MaxSplitscreensPerConnection=1 (DefaultGame.ini [/Script/Engine.GameSession])
pub const SESSION_MAX_SPLITSCREENS: i32 = 1;

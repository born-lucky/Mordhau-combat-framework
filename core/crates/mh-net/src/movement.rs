//! Character movement network prediction: UCharacterMovementComponent's client-side prediction with saved moves, the
//! server's ServerMove processing with time-discrepancy detection / resolution and error correction, and the client's
//! acknowledgement / adjustment / replay, on top of mh-character's exe-exact ExeMovement. Engine code (UE 4.26,
//! statically linked in the exe) and Mordhau's overrides; every rule below cites the exe function it was read from:
//! disassembled with capstone (scripts/ue_dis.py) where the engine body was needed, and the byte-matched Mordhau
//! overrides in src/Mordhau/Private/Components/AdvancedCharacterMovement.cpp (match r9: UAdvancedCharacterMovement::
//! ServerMove_PerformMovement 0x14a0230, ::ProcessClientTimeStampForTimeDiscrepancy 0x149b1f0) are the oracle for the
//! Mordhau layer.
//!
//! The model, per frame:
//!   owning client (ROLE_AutonomousProxy): inputs -> AMordhauCharacter::LODTick -> UMordhauMovementComponent::LODTick
//!     (locally controlled: input vector, MovementModifier) -> [replay the unacknowledged saved moves if the server
//!     corrected us: ClientUpdatePositionAfterServerUpdate] -> ReplicateMoveToServer: a saved move (timestamp, delta,
//!     rounded acceleration, compressed flags, control yaw), PerformMovement with it, ServerMove to the server
//!   server: every received ServerMove in TickDispatch (before the tick groups: session.rs), ServerMove_PerformMovement
//!     -> MoveAutonomous with the client's acceleration and flags, ServerMoveHandleClientError; then the pawn's ticks;
//!     at TickFlush SendClientAdjustment: ClientAckGoodMove or ClientAdjustPosition
//!   client on ClientAckGoodMove: drop the acknowledged moves; on ClientAdjustPosition: drop them, take the server's
//!     location / velocity / mode, and replay the remaining moves next frame
//!
//! Corrections while sprinting (exe behaviour, traced in r4): UMordhauMovementComponent::LODTick, which advances
//! SprintTime (0x1414c79f7), is called from UAdvancedCharacterMovement::TickComponent rva=0x14a3e80 (vcall +0xb20 at
//! 0x1414a41c7, no role test) before UCharacterMovementComponent::TickComponent (call at 0x1414a4337): the owning client
//! ticks it right before its move; the server receives ServerMove in TickDispatch (before the tick groups:
//! UWorld::Tick 0x31b0df0) and ticks the component after, so a server move uses the previous frame's SprintTime and the
//! sprint speed ramp lags a frame there; walking, strafing, turning and jumping replay exactly
//! (tests/movement.rs). A stale (overtaken) ServerMove is dropped by its timestamp and the next move covers the gap
//! with one longer step (capped at MaxMoveDeltaTime), which also produces corrections under jitter.
//!
//! Ported in r4: move combining (FSavedMove_Custom::CanCombineWith 0x14b4a70 + CombineWith 0x2f74d30), the send-rate
//! throttle (GetClientNetSendDeltaTime 0x2f792a0, CanDelaySendingMove 0x2f71bb0), ServerMoveOld (the oldest important
//! unacknowledged move, IsImportantMove 0x2f7b100) and ServerMoveDual, simulated proxies (ProxyMove: ReplicatedMovement,
//! SmoothCorrection, SmoothClientPosition with the CMC constructor's constants).
//! Ported in r5: ReplicatedMovement at a pawn's FRepMovement quantization (quant.rs: Location 1/100 cm from the APawn
//! ctor, velocity whole cm/s, yaw byte); a proxy's SimulateMovement (UAdvancedCharacterMovement 0x14a1e00 -> engine
//! 0x2f8adf0: MoveSmooth / FindFloor / falling / landing on the client's own collision, e.g. mh_level's
//! CollisionWorld); horses (FSavedMove_Horse: the gear in the flags, SetMoveFor 0x14d5970 / PrepMoveFor 0x14d0930,
//! UHorseMovementComponent::UpdateFromCompressedFlags 0x14dd580; the control yaw in the move, PhysicsRotation turning
//! the actor); the movement side of a networked climb (climb_drive).
//! Not ported: movement bases that move (ReplicatedBasedMovement: AAdvancedCharacter::OnRep_ReplicatedBasedMovement
//! 0x1491c10 only forwards to ACharacter's; the levels' collision is static, so a pawn's base never moves and the
//! engine replicates no relative location for it - GatherCurrentMovement uses the based form only for a movable base);
//! root motion (no Mordhau function references root motion - no RootMotion symbol among the game functions - and
//! the one package animation with bEnableRootMotion is a raw clip, Mordhau/Animations/RawClips/Misc/Stun_V2: the
//! motions move the capsule through the movement component, so no root-motion montage replication is needed;
//! UNCONFIRMED that no montage plays that clip); MAXCLIENTUPDATEINTERVAL forced updates; the timestamp reset
//! (MinTimeBetweenTimeStampResets); the owning client's mesh smoothing of its own corrections
//! (UAdvancedCharacterMovement::UsesNetSmoothing 0x14aa780; `smooth_offset` keeps the offset for a renderer).

use std::collections::VecDeque;

use mh_character::exe_cmc;
use mh_character::uemath::{clamped_to_max_size, size_sq, sub};
use mh_character::{ExeInput, ExeMovement, Mode, Sprint, World};
use mordhau_core::ue::FVector;
use serde::{Deserialize, Serialize};

// ---- configuration ----------------------------------------------------------------------------------------------------

/// AGameNetworkManager config ([/Script/Engine.GameNetworkManager], extract/config/BaseGame.ini overridden by
/// DefaultGame.ini) and the field offsets the exe reads them at
#[derive(Clone, Copy, Debug)]
pub struct NetMoveCfg {
    /// MAXPOSITIONERRORSQUARED=3.0f (BaseGame.ini); AGameNetworkManager +0x264, read by ExceedsAllowablePositionError
    /// rva=0x30d2a30 (`comiss xmm6, [rax+0x264]`: LocDiff.SizeSquared() > it)
    pub max_position_error_squared: f32,
    /// MaxMoveDeltaTime=0.125f (BaseGame.ini); FNetworkPredictionData_Server_Character +0x7c (GetServerMoveDeltaTime
    /// rva=0x2f7a800 `mulss xmm2, [rcx+0x7c]`)
    pub max_move_delta_time: f32,
    /// bMovementTimeDiscrepancyDetection=True (DefaultGame.ini); +0x2ac
    pub discrepancy_detection: bool,
    /// bMovementTimeDiscrepancyResolution=True (DefaultGame.ini); +0x2ad
    pub discrepancy_resolution: bool,
    /// MovementTimeDiscrepancyMaxTimeMargin=0.25 (DefaultGame.ini); +0x2b0
    pub discrepancy_max_margin: f32,
    /// MovementTimeDiscrepancyMinTimeMargin=-0.25 (DefaultGame.ini); +0x2b4
    pub discrepancy_min_margin: f32,
    /// MovementTimeDiscrepancyResolutionRate=1.0 (DefaultGame.ini); +0x2b8
    pub discrepancy_resolution_rate: f32,
    /// MovementTimeDiscrepancyDriftAllowance=0.05 (DefaultGame.ini); +0x2bc
    pub discrepancy_drift_allowance: f32,
    /// bMovementTimeDiscrepancyForceCorrectionsDuringResolution=false (BaseGame.ini); +0x2c0
    pub discrepancy_force_corrections: bool,
    /// UAdvancedCharacterMovement::ProcessClientTimeStampForTimeDiscrepancy 0x149b1f0: no discrepancy tracking during
    /// the first 5 s after possession (`RealTimeSeconds < LastPossessionTime + 5.f`, src/.../AdvancedCharacterMovement.cpp)
    pub possession_grace: f32,
    /// UCharacterMovementComponent ctor rva=0x2f6cc10: NetworkMinTimeBetweenClientAckGoodMoves 0.1 (+0x2d8,
    /// 0x142f6d14b), NetworkMinTimeBetweenClientAdjustments 0.1 (+0x2dc, 0x142f6d155),
    /// NetworkMinTimeBetweenClientAdjustmentsLargeCorrection 0.05 (+0x2e0, 0x142f6d15f), NetworkLargeClientCorrection-
    /// Distance 15 (+0x2e4, 0x142f6d169); no Mordhau ctor, Blueprint or ini changes them
    pub min_time_between_ack_good_moves: f32,
    pub min_time_between_adjustments: f32,
    pub min_time_between_adjustments_large: f32,
    pub large_client_correction_distance: f32,
}

impl Default for NetMoveCfg {
    fn default() -> Self {
        NetMoveCfg {
            max_position_error_squared: 3.0,
            max_move_delta_time: 0.125,
            discrepancy_detection: true,
            discrepancy_resolution: true,
            discrepancy_max_margin: 0.25,
            discrepancy_min_margin: -0.25,
            discrepancy_resolution_rate: 1.0,
            discrepancy_drift_allowance: 0.05,
            discrepancy_force_corrections: false,
            possession_grace: 5.0,
            min_time_between_ack_good_moves: 0.1,
            min_time_between_adjustments: 0.1,
            min_time_between_adjustments_large: 0.05,
            large_client_correction_distance: 15.0,
        }
    }
}

/// 1e-6 floor of a resolution-mode move delta (ProcessClientTimeStampForTimeDiscrepancy `maxss xmm0, [0x144000104]`)
pub const MIN_TICK_TIME: f32 = 1e-6;

// ---- wire forms ---------------------------------------------------------------------------------------------------------

/// UCharacterMovementComponent::RoundAcceleration rva=0x2f85650 (vtable +0x970 of UMordhauMovementComponent, called by
/// FSavedMove_Character::SetMoveFor rva=0x2f89a40 at 0x142f89b32): each component Floor(X * 10 + 0.5) * 0.1 in binary32
/// (`mulss 10` .rdata 0x143fe4e24, `addss 0.5` 0x143fe4e04, cvttss2si + the negative-fraction adjust = FloorToFloat,
/// `mulss 0.1` 0x143fe4dfc); an out-of-range cvttss2si (0x80000000) keeps X * 10 + 0.5 unfloored. The same precision
/// as FVector_NetQuantize10, so the server sees exactly what the client simulated.
pub fn round_acceleration(a: FVector) -> FVector {
    fn r(x: f32) -> f32 {
        let y = x * 10.0 + 0.5;
        let t = y as i64; // cvttss2si (range checked below)
        if !(-2147483648.0..2147483648.0).contains(&y) {
            return y * 0.1;
        }
        let mut i = t as i32;
        if i as f32 != y && y < 0.0 {
            i -= 1; // movmskps sign bit: truncation of a negative non-integer rounds up, so step down
        }
        i as f32 * 0.1
    }
    FVector { x: r(a.x), y: r(a.y), z: r(a.z) }
}

/// FVector_NetQuantize100, the precision of FCharacterNetworkMoveData::Location: SerializePackedVector<100,30>
/// rva=0x2f6bf60 (WritePackedVector<100,30> 0x2f6c380, read * 0.01f), quant.rs
pub fn quantize100(a: FVector) -> FVector {
    crate::quant::net_quantize100(a)
}

/// FRotator::CompressAxisToShort / DecompressAxisFromShort (the packed control rotation of ServerMove), in the form
/// FRotator::SerializeCompressedShort rva=0x18bf4e0 computes them (quant.rs: RoundToInt(Angle * 182.04445f) & 0xFFFF,
/// Short * 0.0054931640625f). UNCONFIRMED: the ServerMove packing site itself (inlined into the callers) is not traced.
pub fn compress_axis(yaw: f32) -> u16 {
    crate::quant::compress_axis_to_short(yaw)
}
pub fn decompress_axis(s: u16) -> f32 {
    crate::quant::decompress_axis_from_short(s)
}

/// FSavedMove_Custom::GetCompressedFlags rva=0x14bc4c0: bit 0 bPressedJump, bit 1 bWantsToCrouch, bits 4..6 the three
/// bits of EMovementModifier (UMordhauMovementComponent +0xd18, mh-character Sprint)
pub fn compressed_flags(pressed_jump: bool, wants_to_crouch: bool, modifier: u8) -> u8 {
    (pressed_jump as u8) | ((wants_to_crouch as u8) << 1) | ((modifier >> 2 & 1) << 6) | ((modifier >> 1 & 1) << 5) | ((modifier & 1) << 4)
}

fn sprint_of(m: u8) -> Sprint {
    match m & 7 {
        0 => Sprint::Forward,
        1 => Sprint::Sideways,
        2 => Sprint::Backpedal,
        3 => Sprint::Partial,
        4 => Sprint::Sprint,
        5 => Sprint::Rush,
        6 => Sprint::Chase,
        _ => Sprint::Super,
    }
}

/// UCharacterMovementComponent::PackNetworkMovementMode rva=0x2f7de20 (vtable +0x5b8): the walking modes as their
/// EMovementMode value (no custom modes here)
pub fn pack_mode(m: Mode) -> u8 {
    m as u8
}
fn unpack_mode(b: u8) -> Mode {
    match b {
        1 => Mode::Walking,
        2 => Mode::NavWalking,
        3 => Mode::Falling,
        6 => Mode::Custom,
        _ => Mode::None,
    }
}

/// ServerMove's FCharacterNetworkMoveData (TimeStamp, Acceleration NetQuantize10, Location NetQuantize100,
/// CompressedMoveFlags, ControlRotation packed, MovementMode)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServerMoveData {
    pub time_stamp: f32,
    pub accel: [f32; 3],
    pub location: [f32; 3],
    pub flags: u8,
    pub yaw: u16,
    pub mode: u8,
}

/// the server's answer to a move (SendClientAdjustment rva=0x2f85d00)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MoveResponse {
    /// ClientAckGoodMove_Implementation rva=0x2f73070
    AckGoodMove { time_stamp: f32 },
    /// ClientAdjustPosition_Implementation rva=0x2f73200
    AdjustPosition { time_stamp: f32, location: [f32; 3], velocity: [f32; 3], mode: u8 },
}

fn a3(v: FVector) -> [f32; 3] {
    [v.x, v.y, v.z]
}
fn v3(a: [f32; 3]) -> FVector {
    FVector { x: a[0], y: a[1], z: a[2] }
}

// ---- shared: MoveAutonomous ---------------------------------------------------------------------------------------------

/// UMordhauMovementComponent::UpdateFromCompressedFlags rva=0x14dd5f0: the CMC part (bPressedJump = bit 0 - a new
/// press resets bWasJumping / JumpKeyHoldTime -, bWantsToCrouch = bit 1), then MovementModifier = bits 4..6; a horse's
/// UHorseMovementComponent::UpdateFromCompressedFlags rva=0x14dd580: the CMC part (call 0x142f921d0), then Gear = bits
/// 4..5 and ValidateGear(5) (exe_horse.rs)
pub fn update_from_compressed_flags(m: &mut ExeMovement, flags: u8) {
    let was = m.pressed_jump;
    m.pressed_jump = flags & 1 != 0;
    m.wants_to_crouch = flags & 2 != 0;
    if !was && m.pressed_jump {
        m.was_jumping = false;
        m.jump_key_hold_time = 0.0;
    }
    if m.horse.is_some() {
        m.horse_update_from_compressed_flags(flags);
        return;
    }
    m.sprint_state = sprint_of((flags >> 6 & 1) << 2 | (flags >> 5 & 1) << 1 | (flags >> 4 & 1));
}

/// the controller's ControlRotation yaw a saved move carries: a character's actor yaw follows it directly
/// (bUseControllerRotationYaw), a horse's turns toward it in PhysicsRotation (exe_horse.rs)
pub fn control_yaw(m: &ExeMovement) -> f32 {
    m.horse.as_ref().map_or(m.yaw, |h| h.control_yaw)
}

/// SetControlRotation of a move (ServerMove_PerformMovement / the replay's PrepMoveFor)
pub fn set_control_yaw(m: &mut ExeMovement, yaw: f32) {
    match m.horse.as_mut() {
        Some(h) => h.control_yaw = yaw,
        None => m.set_yaw(yaw),
    }
}

/// UCharacterMovementComponent::MoveAutonomous rva=0x2f7c5c0 (calls read off the disassembly): HasValidData (+0x5d8),
/// UpdateFromCompressedFlags (+0x9c8), the owner's CheckJumpInput (+0x840), ConstrainInputAcceleration (+0x918),
/// GetClampedToMaxSize(GetMaxAcceleration (+0x6e8)) (the 1e-4 compare .rdata 0x144022350), ComputeAnalogInputModifier
/// (+0x5a0), PerformMovement (UAdvancedCharacterMovement::PerformMovement 0x1495cc0: + CheckFallDamage(Velocity.Z))
pub fn move_autonomous(m: &mut ExeMovement, world: &dyn World, dt: f32, flags: u8, accel: FVector) {
    update_from_compressed_flags(m, flags);
    m.check_jump_input(world);
    let a = m.constrain_input_acceleration(accel);
    m.acceleration = clamped_to_max_size(a, m.get_max_acceleration());
    m.analog_input_modifier = m.compute_analog_input_modifier();
    m.perform_movement(world, dt);
    let vz = m.velocity.z;
    m.check_fall_damage(vz);
}

// ---- client ------------------------------------------------------------------------------------------------------------

/// FSavedMove_Character thresholds, written by its constructor rva=0x2f6caa0: AccelMagThreshold 1 (+0x29c),
/// AccelDotThreshold 0.9 (+0x298), AccelDotThresholdCombine 0.996 (+0x2a0), MaxSpeedThresholdCombine 10 (+0x2a4)
pub const ACCEL_MAG_THRESHOLD: f32 = 1.0;
pub const ACCEL_DOT_THRESHOLD: f32 = 0.9;
pub const ACCEL_DOT_THRESHOLD_COMBINE: f32 = 0.996;
pub const MAX_SPEED_THRESHOLD_COMBINE: f32 = 10.0;

/// FSavedMove_Custom (FSavedMove_Character + MovementModifier): the fields the replay, the combining and the
/// important-move test read
#[derive(Clone, Debug)]
pub struct SavedMove {
    pub time_stamp: f32,
    pub delta_time: f32,
    /// RoundAcceleration of the input acceleration (SetMoveFor)
    pub accel: FVector,
    /// AccelMag / AccelNormal of the unrounded acceleration (SetMoveFor computes them before rounding)
    pub accel_mag: f32,
    pub accel_normal: FVector,
    pub flags: u8,
    pub yaw: f32,
    /// FSavedMove_Horse SavedGear (+0x2a8; 0 for a character)
    pub gear: u8,
    /// MaxSpeed (GetMaxSpeed at SetMoveFor)
    pub max_speed: f32,
    pub start_location: FVector,
    pub start_velocity: FVector,
    pub start_floor: mh_character::FloorResult,
    pub start_mode: u8,
    pub end_mode: u8,
    pub jump_key_hold_time: f32,
    pub was_jumping: bool,
    pub jump_current_count: i32,
    pub jump_force_time_remaining: f32,
    /// SavedLocation / SavedVelocity after the move (PostUpdate)
    pub saved_location: FVector,
    pub saved_velocity: FVector,
}

impl SavedMove {
    /// FSavedMove_Character::SetMoveFor rva=0x2f89a40 + FSavedMove_Custom::SetMoveFor 0x14d5900 (MovementModifier) or
    /// FSavedMove_Horse::SetMoveFor rva=0x14d5970 (the gear block, then SavedGear): the start state; the flags are
    /// GetCompressedFlags of it (FSavedMove_Custom 0x14bc4c0 / FSavedMove_Horse 0x14bc510)
    fn set_move_for(m: &mut ExeMovement, ts: f32, dt: f32, accel: FVector) -> Self {
        let gear = if m.horse.is_some() { m.horse_set_move_for_gear() } else { 0 };
        let flags = if m.horse.is_some() { (m.pressed_jump as u8) | ((m.wants_to_crouch as u8) << 1) | m.horse_compressed_gear_bits() } else { compressed_flags(m.pressed_jump, m.wants_to_crouch, m.sprint_state as u8) };
        let mag2 = size_sq(accel);
        let mag = mag2.sqrt();
        let normal = if mag2 > 1e-8 { mul_v(accel, 1.0 / mag) } else { FVector::ZERO };
        SavedMove {
            time_stamp: ts,
            delta_time: dt,
            accel: round_acceleration(accel),
            accel_mag: mag,
            accel_normal: normal,
            flags,
            yaw: control_yaw(m),
            gear,
            max_speed: m.get_max_speed(),
            start_location: m.location,
            start_velocity: m.velocity,
            start_floor: m.current_floor,
            start_mode: pack_mode(m.mode),
            end_mode: pack_mode(m.mode),
            jump_key_hold_time: m.jump_key_hold_time,
            was_jumping: m.was_jumping,
            jump_current_count: m.jump_current_count,
            jump_force_time_remaining: m.jump_force_time_remaining,
            saved_location: m.location,
            saved_velocity: m.velocity,
        }
    }

    fn data(&self) -> ServerMoveData {
        ServerMoveData {
            time_stamp: self.time_stamp,
            accel: a3(self.accel),
            location: a3(quantize100(self.saved_location)),
            flags: self.flags,
            yaw: compress_axis(self.yaw),
            mode: self.end_mode,
        }
    }

    /// FSavedMove_Character::IsImportantMove rva=0x2f7b100 (against the last acknowledged move): other compressed
    /// flags (vcall +0x50), StartPackedMovementMode (+0x39) or EndPackedMovementMode (+0x168) != the acknowledged
    /// move's EndPackedMovementMode; acceleration changed: |AccelMag - its AccelMag| > AccelMagThreshold or
    /// AccelNormal | its AccelNormal < AccelDotThreshold
    pub fn is_important_move(&self, acked: &SavedMove) -> bool {
        if self.flags != acked.flags || self.start_mode != acked.end_mode || self.end_mode != acked.end_mode {
            return true;
        }
        if self.accel == acked.accel {
            return false;
        }
        if (self.accel_mag - acked.accel_mag).abs() > ACCEL_MAG_THRESHOLD {
            return true;
        }
        dot_v(self.accel_normal, acked.accel_normal) < ACCEL_DOT_THRESHOLD
    }

    /// FSavedMove_Custom::CanCombineWith rva=0x14b4a70 (this = the pending move): the same MovementModifier; no
    /// bForceNoCombine; a zero new acceleration only with a zero pending one, else DeltaTime sum < MaxDelta and
    /// AccelNormal dot >= AccelDotThresholdCombine; StartVelocity zero-ness equal; |MaxSpeed diff| <=
    /// MaxSpeedThresholdCombine and its zero-ness equal; JumpKeyHoldTime zero-ness, bWasJumping, JumpCurrentCount,
    /// JumpForceTimeRemaining zero-ness equal; equal compressed flags; StartPackedMovementMode equal and the pending
    /// move's EndPackedMovementMode == the new StartPackedMovementMode. Bases, attachment, root motion, capsule size:
    /// none here (they are equal).
    pub fn can_combine_with(&self, new: &SavedMove, max_delta: f32) -> bool {
        if self.flags & 0x70 != new.flags & 0x70 {
            return false;
        }
        if new.accel == FVector::ZERO {
            if self.accel != FVector::ZERO {
                return false;
            }
        } else {
            if max_delta <= new.delta_time + self.delta_time {
                return false;
            }
            if dot_v(self.accel_normal, new.accel_normal) < ACCEL_DOT_THRESHOLD_COMBINE {
                return false;
            }
        }
        if (self.start_velocity == FVector::ZERO) != (new.start_velocity == FVector::ZERO) {
            return false;
        }
        let d = (self.max_speed - new.max_speed).abs();
        if MAX_SPEED_THRESHOLD_COMBINE < d {
            return false;
        }
        if (self.max_speed == 0.0) != (new.max_speed == 0.0) {
            return false;
        }
        if (self.jump_key_hold_time == 0.0) != (new.jump_key_hold_time == 0.0) || self.was_jumping != new.was_jumping {
            return false;
        }
        if self.jump_current_count != new.jump_current_count || (self.jump_force_time_remaining == 0.0) != (new.jump_force_time_remaining == 0.0) {
            return false;
        }
        if self.flags != new.flags {
            return false;
        }
        self.start_mode == new.start_mode && self.end_mode == new.start_mode
    }
}

fn mul_v(a: FVector, s: f32) -> FVector {
    FVector { x: a.x * s, y: a.y * s, z: a.z * s }
}
fn dot_v(a: FVector, b: FVector) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// AGameNetworkManager send-rate settings ([/Script/Engine.GameNetworkManager]: BaseGame.ini, DefaultGame.ini), as
/// UCharacterMovementComponent::GetClientNetSendDeltaTime rva=0x2f792a0 reads them (offsets of this build)
#[derive(Clone, Copy, Debug)]
pub struct SendCfg {
    /// ClientNetSendMoveDeltaTime=0.0166 (BaseGame.ini; +0x288)
    pub delta_time: f32,
    /// ClientNetSendMoveDeltaTimeThrottled=0.0333 (DefaultGame.ini; +0x28c)
    pub delta_time_throttled: f32,
    /// ClientNetSendMoveDeltaTimeStationary=0.0333 (DefaultGame.ini; +0x290)
    pub delta_time_stationary: f32,
    /// ClientNetSendMoveThrottleAtNetSpeed=10000 (BaseGame.ini; +0x294)
    pub throttle_at_net_speed: i32,
    /// ClientNetSendMoveThrottleOverPlayerCount=48 (DefaultGame.ini; +0x298)
    pub throttle_over_player_count: i32,
    /// MoveRepSize=42.0f (BaseGame.ini; +0x260)
    pub move_rep_size: f32,
    /// the connection's CurrentNetSpeed: MaxInternetClientRate=100000 ([/Script/OnlineSubsystemUtils.IpNetDriver]
    /// BaseEngine.ini / DefaultEngine.ini) (UNCONFIRMED: the engine's net speed negotiation is not traced)
    pub net_speed: i32,
    /// GameState PlayerArray.Num()
    pub player_count: i32,
    /// CharacterMovementCVars::NetEnableMoveCombining (engine default 1; UNCONFIRMED value in this build)
    pub enable_move_combining: bool,
}

impl Default for SendCfg {
    fn default() -> Self {
        SendCfg {
            delta_time: 0.0166,
            delta_time_throttled: 0.0333,
            delta_time_stationary: 0.0333,
            throttle_at_net_speed: 10000,
            throttle_over_player_count: 48,
            move_rep_size: 42.0,
            net_speed: 100000,
            player_count: 2,
            enable_move_combining: true,
        }
    }
}

/// FNetworkPredictionData_Client_Character, the parts ported
#[derive(Clone, Debug, Default)]
pub struct ClientPrediction {
    /// CurrentTimeStamp (UpdateTimeStampAndDeltaTime: += DeltaTime)
    pub current_time_stamp: f32,
    /// SavedMoves, oldest first
    pub saved_moves: VecDeque<SavedMove>,
    /// PendingMove: a move held back to be combined with the next or sent with it (ServerMoveDual)
    pub pending: Option<SavedMove>,
    /// LastAckedMove
    pub last_acked: Option<SavedMove>,
    /// ClientUpdateTime: world time of the last ServerMove sent
    pub client_update_time: f32,
    pub send: SendCfg,
    /// bUpdatePosition: replay before the next move (ClientUpdatePositionAfterServerUpdate rva=0x2f746a0)
    pub update_position: bool,
    pub acks: u32,
    pub corrections: u32,
    pub replays: u32,
    pub combined: u32,
    pub packets: u32,
    pub old_moves_sent: u32,
    pub duals_sent: u32,
    /// the last correction's offset (old location - new location), for a renderer's mesh smoothing
    pub smooth_offset: FVector,
}

impl ClientPrediction {
    /// FNetworkPredictionData_Client_Character::GetSavedMoveIndex rva=0x2f7a7a0
    pub fn saved_move_index(&self, ts: f32) -> Option<usize> {
        self.saved_moves.iter().position(|m| m.time_stamp == ts)
    }

    /// FNetworkPredictionData_Client_Character::AckMove rva=0x2f6d7b0: LastAckedMove = it; it and every older one go
    fn ack_move(&mut self, idx: usize) {
        self.last_acked = Some(self.saved_moves[idx].clone());
        self.saved_moves.drain(..=idx);
    }

    /// UCharacterMovementComponent::GetClientNetSendDeltaTime rva=0x2f792a0 (structure read off the disassembly:
    /// ClientNetSendMoveDeltaTime; when CurrentNetSpeed <= ThrottleAtNetSpeed or players > ThrottleOverPlayerCount:
    /// max(Throttled, 2 x MoveRepSize / CurrentNetSpeed); standing still (zero Acceleration and Velocity) with an
    /// acknowledged move at the same control rotation: max(Stationary, it)); the caller clamps to [1/120, 1/5]
    pub fn client_net_send_delta_time(&self, m: &ExeMovement) -> f32 {
        let c = &self.send;
        let mut d = c.delta_time;
        if !(c.net_speed > c.throttle_at_net_speed && c.player_count <= c.throttle_over_player_count) {
            d = c.delta_time_throttled.max(2.0 * c.move_rep_size / c.net_speed as f32);
        }
        if m.acceleration == FVector::ZERO && m.velocity == FVector::ZERO && self.last_acked.as_ref().is_some_and(|a| a.yaw == control_yaw(m)) {
            d = c.delta_time_stationary.max(d);
        }
        d.clamp(1.0 / 120.0, 1.0 / 5.0)
    }
}

/// what a client sends for one or more moves: ServerMoveOld (the oldest important unacknowledged move, re-sent so a
/// lost datagram cannot drop it), ServerMoveDual (a held-back pending move that could not be combined, with the new
/// one) or ServerMove
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ServerMovePacket {
    pub old: Option<OldMoveData>,
    pub pending: Option<ServerMoveData>,
    pub new: ServerMoveData,
}

/// ServerMoveOld(OldTimeStamp, OldAccel, OldMoveFlags)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct OldMoveData {
    pub time_stamp: f32,
    pub accel: [f32; 3],
    pub flags: u8,
    /// FCharacterNetworkMoveData::Serialize rva=0x2f866b0 writes Location and ControlRotation for every move type
    /// (only the base / bone / mode are NewMove-only): the old move's saved location (NetQuantize100) and control yaw
    #[serde(default)]
    pub location: [f32; 3],
    #[serde(default)]
    pub yaw: u16,
}

/// One frame of the owning client's pawn: the same steps as ExeMovement::frame (kept in step with mh-character's;
/// tests/movement.rs checks that a client_frame with no network equals ExeMovement::frame), with the move recorded,
/// simulated with the acceleration the server will see, and, when it is time to send, returned for ServerMove.
pub fn client_frame(m: &mut ExeMovement, cp: &mut ClientPrediction, world: &dyn World, dt: f32, inp: &ExeInput) -> Option<ServerMovePacket> {
    client_frame_with(m, cp, world, dt, inp, &mut |_| {})
}

/// client_frame with the rest of the actor tick as a hook: `actor_tick` runs after the input and
/// AMordhauCharacter::LODTick's movement parts, before the movement component's tick, where the actor tick's
/// motion-system tick runs in the exe (UMotionSystemComponent::OnLODTick rva=0x14cb3e0 is bound to the character's
/// LODTick delegate in OnRegister rva=0x14cde00; the component itself does not tick: its ctor rva=0x14afca0 clears
/// bCanEverTick). The node starts a requested climb there and steps the running one (climb_drive), so the move it then
/// saves carries this frame's climb position, as the exe's does.
pub fn client_frame_with(m: &mut ExeMovement, cp: &mut ClientPrediction, world: &dyn World, dt: f32, inp: &ExeInput, actor_tick: &mut dyn FnMut(&mut ExeMovement)) -> Option<ServerMovePacket> {
    m.world_time += dt;
    if inp.sprint != m.sprint_held {
        if inp.sprint {
            m.sprint_pressed();
        } else {
            m.sprint_released();
        }
        m.sprint_held = inp.sprint;
    }
    if inp.crouch != m.crouch_held {
        if inp.crouch {
            m.crouch_pressed();
        } else {
            m.crouch_released();
        }
        m.crouch_held = inp.crouch;
    }
    // edge-triggered Jump (AMordhauCharacter::JumpPressed: TryClimbing, then Jump / JumpReleased: StopJumping), before
    // this frame's yaw, as ExeMovement::frame (mh-character r4)
    m.jump_input(world, inp.jump);
    m.set_yaw(inp.yaw);
    let (_, fwd_axis, right_axis) = mh_character::uequat::actor_axes(inp.yaw);
    m.move_forward(inp.fwd, fwd_axis);
    m.move_right(inp.right, right_axis);
    m.character_lod_tick(dt);
    m.climb_retry(world); // bWantsClimb retry (AMordhauCharacter::LODTick 0x14154c556)
    actor_tick(m);
    // UAdvancedCharacterMovement::TickComponent rva=0x14a3e80: UMordhauMovementComponent::LODTick (vcall +0xb20 at
    // 0x1414a41c7, every role) before UCharacterMovementComponent::TickComponent (call at 0x1414a4337)
    m.lod_tick(dt);
    // TickComponent start: a correction received since the last frame -> replay (ClientUpdatePositionAfterServerUpdate)
    if cp.update_position {
        client_update_position_after_server_update(m, cp, world);
    }
    let input = m.consume_input_vector();
    // TickComponent (autonomous proxy): CheckJumpInput, Acceleration = ScaleInputAcceleration(ConstrainInput...)
    m.check_jump_input(world);
    let a = m.scale_input_acceleration(m.constrain_input_acceleration(input));
    let pkt = replicate_move_to_server(m, cp, world, dt, a, &NetMoveCfg::default());
    m.tick_ragdoll_get_up(dt);
    m.post_character_movement_tick(dt);
    pkt
}

/// One frame of the owning client's horse (a pawn its PlayerController possesses: UHorseMovementComponent, saved
/// moves FSavedMove_Horse): the order of ExeMovement::horse_frame (the Jump action, AHorse::MoveForward / MoveRight,
/// AHorse::LODTick, UHorseMovementComponent::LODTick) with the movement tick replaced by the prediction (replay after a
/// correction, CheckJumpInput, the input acceleration, ReplicateMoveToServer); `h.controlled` must be true. Rearing
/// requests (AHorse::RequestRearing -> ServerRequestRearing) are counted in `rear_requests` for the host to send.
pub fn client_frame_horse(m: &mut ExeMovement, cp: &mut ClientPrediction, world: &dyn World, dt: f32, inp: &mh_character::exe_horse::HorseInput) -> Option<ServerMovePacket> {
    m.world_time += dt;
    let held = m.horse.as_ref().is_some_and(|h| h.jump_held);
    if inp.jump != held {
        if let Some(h) = m.horse.as_mut() {
            h.jump_held = inp.jump;
        }
        if inp.jump {
            m.horse_jump_pressed();
        }
    }
    m.horse_move_forward(inp.fwd);
    m.horse_move_right(inp.right);
    m.horse_character_lod_tick();
    m.horse_lod_tick(world, dt);
    if cp.update_position {
        client_update_position_after_server_update(m, cp, world);
    }
    let input = m.consume_input_vector();
    m.check_jump_input(world);
    let a = m.scale_input_acceleration(m.constrain_input_acceleration(input));
    let pkt = replicate_move_to_server(m, cp, world, dt, a, &NetMoveCfg::default());
    m.tick_ragdoll_get_up(dt);
    // UHorseMovementComponent::PostCharacterMovementTick rva=0xb93a60: empty
    pkt
}

/// The server's frame for a horse nobody on this machine drives (no ServerMoves): ExeMovement::horse_frame without the
/// clock and with no input - AHorse::LODTick (not controlled: DesiredGear = UncontrolledGear), UHorseMovementComponent::
/// LODTick (the gear block on the authority), the movement tick
pub fn server_uncontrolled_horse_tick(m: &mut ExeMovement, world: &dyn World, dt: f32) {
    m.horse_move_forward(0.0);
    m.horse_move_right(0.0);
    m.horse_character_lod_tick();
    m.horse_lod_tick(world, dt);
    let input = m.consume_input_vector();
    m.controlled_character_move(world, input, dt);
    m.tick_ragdoll_get_up(dt);
}

// ---- climbing (the movement side of a networked UClimbingMotion) ------------------------------------------------------

/// A climb the movement side is running (mh-sim climb.rs's ClimbRun on a networked machine)
#[derive(Clone, Debug, Default)]
pub struct NetClimb {
    pub run: Option<((u32, u32), mh_character::exe_climb::ClimbState, mh_character::exe_climb::ClimbTiming)>,
}

/// After the combat step on a machine that moves the pawn - the authority, or the owning client (UClimbingMotion::
/// OnTick_Implementation rva=0x1667790 runs on role >= autonomous proxy only, 0x1416677dd..0x1416677ed): a new current
/// UClimbingMotion -> its OnBegin movement part (climb_on_begin: MOVE_Custom, ledge offset, timing); while current ->
/// climb_step toward ClimbTargetLocation (+0xdf8: on the owning client the target its AttemptClimb found, on the server
/// the one ServerSetClimbLocation delivered, a FVector_NetQuantize - whole cm: BP_MordhauCharacter's
/// ServerSetClimbLocation(NewParam: Vector_NetQuantize), package Mordhau/Content/Mordhau/Blueprints/Characters/
/// BP_MordhauCharacter, FUNC_NetReliable | FUNC_NetServer); when it is no longer current -> MOVE_Walking
/// (OnLeave_Implementation rva=0x1665780). Simulated proxies see the result through ReplicatedMovement (MOVE_Custom:
/// MoveSmooth's plain swept move).
pub fn climb_drive(m: &mut ExeMovement, world: &dyn World, cur: Option<crate::world::ClimbView>, c: &mut NetClimb) {
    match cur {
        Some(v) => {
            if c.run.as_ref().map(|r| r.0) != Some(v.key) {
                let b = m.climb_on_begin(world, &v.data, v.params, v.start_time);
                c.run = Some((v.key, mh_character::exe_climb::ClimbState::default(), b.timing));
            }
            let target = m.climb_target_location;
            if let Some((_, st, tm)) = c.run.as_mut() {
                mh_character::exe_climb::climb_step(m, st, tm, target);
            }
        }
        None => {
            if c.run.take().is_some() {
                m.set_movement_mode(world, Mode::Walking);
            }
        }
    }
}

/// UCharacterMovementComponent::ReplicateMoveToServer rva=0x2f83ff0: CurrentTimeStamp += DeltaTime; a new saved move
/// (SetMoveFor); a held-back PendingMove is combined with it when CanCombineWith (the pawn goes back to the pending
/// move's start - location, velocity, floor, jump state - and the combined move runs over both deltas: CombineWith
/// rva=0x2f74d30), else both go out (ServerMoveDual); Acceleration = the move's GetClampedToMaxSize(MaxAcceleration),
/// PerformMovement, PostUpdate, SavedMoves.Push; then, when the move can be delayed (CanDelaySendingMove
/// rva=0x2f71bb0: Start == End packed movement mode and no bForceNoCombine) and none is pending, it is held back while
/// TimeSeconds - ClientUpdateTime < clamp(GetClientNetSendDeltaTime, 1/120, 1/5); otherwise ClientUpdateTime = now
/// and CallServerMove with the oldest important unacknowledged move (IsImportantMove against LastAckedMove, the
/// newest move excluded) as ServerMoveOld.
pub fn replicate_move_to_server(m: &mut ExeMovement, cp: &mut ClientPrediction, world: &dyn World, dt: f32, accel: FVector, cfg: &NetMoveCfg) -> Option<ServerMovePacket> {
    cp.current_time_stamp += dt;
    let mut new = SavedMove::set_move_for(m, cp.current_time_stamp, dt, accel);
    let mut dual: Option<SavedMove> = None;
    if let Some(p) = cp.pending.take() {
        if cp.send.enable_move_combining && p.can_combine_with(&new, cfg.max_move_delta_time) {
            m.location = p.start_location;
            m.velocity = p.start_velocity;
            m.current_floor = p.start_floor;
            m.jump_current_count = p.jump_current_count;
            m.jump_force_time_remaining = p.jump_force_time_remaining;
            m.was_jumping = p.was_jumping;
            m.jump_key_hold_time = p.jump_key_hold_time;
            new.delta_time += p.delta_time;
            new.start_location = p.start_location;
            new.start_velocity = p.start_velocity;
            new.start_floor = p.start_floor;
            if cp.saved_moves.back().is_some_and(|b| b.time_stamp == p.time_stamp) {
                cp.saved_moves.pop_back();
            }
            cp.combined += 1;
        } else {
            dual = Some(p);
        }
    }
    m.acceleration = clamped_to_max_size(new.accel, m.get_max_acceleration());
    m.analog_input_modifier = m.compute_analog_input_modifier();
    m.perform_movement(world, new.delta_time);
    let vz = m.velocity.z;
    m.check_fall_damage(vz);
    new.saved_location = m.location;
    new.saved_velocity = m.velocity;
    new.end_mode = pack_mode(m.mode);
    cp.saved_moves.push_back(new.clone());
    let can_delay = cp.send.enable_move_combining && new.start_mode == new.end_mode;
    if can_delay && dual.is_none() {
        let d = cp.client_net_send_delta_time(m);
        if m.world_time - cp.client_update_time < d {
            cp.pending = Some(new);
            return None;
        }
    }
    cp.client_update_time = m.world_time;
    let old = cp.last_acked.as_ref().and_then(|la| {
        let n = cp.saved_moves.len();
        cp.saved_moves.iter().take(n.saturating_sub(1)).find(|sm| sm.is_important_move(la)).map(|sm| OldMoveData { time_stamp: sm.time_stamp, accel: a3(sm.accel), flags: sm.flags, location: a3(quantize100(sm.saved_location)), yaw: compress_axis(sm.yaw) })
    });
    cp.packets += 1;
    if old.is_some() {
        cp.old_moves_sent += 1;
    }
    if dual.is_some() {
        cp.duals_sent += 1;
    }
    Some(ServerMovePacket { old, pending: dual.map(|p| p.data()), new: new.data() })
}

/// ClientUpdatePositionAfterServerUpdate rva=0x2f746a0: replay every unacknowledged saved move from the corrected
/// state (PrepMoveFor: FSavedMove_Custom::PrepMoveFor 0x14d08d0 restores MovementModifier; MoveAutonomous), keeping the
/// current jump / crouch / modifier state around it; a pending move is replayed too (it is in SavedMoves)
pub fn client_update_position_after_server_update(m: &mut ExeMovement, cp: &mut ClientPrediction, world: &dyn World) {
    cp.update_position = false;
    let (pj, wc, sp, yaw) = (m.pressed_jump, m.wants_to_crouch, m.sprint_state, control_yaw(m));
    let moves: Vec<SavedMove> = cp.saved_moves.iter().cloned().collect();
    for (i, sm) in moves.iter().enumerate() {
        set_control_yaw(m, sm.yaw);
        if m.horse.is_some() {
            m.horse_prep_move_for_gear(sm.gear); // FSavedMove_Horse::PrepMoveFor rva=0x14d0930
        }
        cp.saved_moves[i].start_location = m.location;
        cp.saved_moves[i].start_velocity = m.velocity;
        cp.saved_moves[i].start_floor = m.current_floor;
        move_autonomous(m, world, sm.delta_time, sm.flags, sm.accel);
        cp.saved_moves[i].saved_location = m.location;
        cp.saved_moves[i].saved_velocity = m.velocity;
        cp.replays += 1;
    }
    if let Some(p) = cp.pending.as_mut() {
        if let Some(last) = cp.saved_moves.back() {
            if last.time_stamp == p.time_stamp {
                *p = last.clone();
            }
        }
    }
    m.pressed_jump = pj;
    m.wants_to_crouch = wc;
    m.sprint_state = sp;
    set_control_yaw(m, yaw);
}

/// the owning client handles the server's answer (ClientAckGoodMove_Implementation rva=0x2f73070: GetSavedMoveIndex,
/// none -> log only, else AckMove; ClientAdjustPosition_Implementation rva=0x2f73200: AckMove, location / velocity /
/// movement mode from the server, UpdateFloorFromAdjustment (UAdvancedCharacterMovement 0x14a9720), bJustTeleported,
/// bUpdatePosition)
pub fn client_handle_response(m: &mut ExeMovement, cp: &mut ClientPrediction, world: &dyn World, r: &MoveResponse) {
    match r {
        MoveResponse::AckGoodMove { time_stamp } => {
            if let Some(i) = cp.saved_move_index(*time_stamp) {
                cp.ack_move(i);
                cp.acks += 1;
            }
        }
        MoveResponse::AdjustPosition { time_stamp, location, velocity, mode } => {
            let Some(i) = cp.saved_move_index(*time_stamp) else { return };
            cp.ack_move(i);
            let old = m.location;
            m.location = v3(*location);
            m.velocity = v3(*velocity);
            let md = unpack_mode(*mode);
            if md != m.mode {
                m.set_movement_mode(world, md);
            }
            m.force_next_floor_check = true; // UpdateFloorFromAdjustment: find the floor at the new location
            m.just_teleported = true;
            m.last_update_location = m.location;
            cp.smooth_offset = sub(old, m.location);
            cp.update_position = true;
            cp.corrections += 1;
        }
    }
}

// ---- simulated proxies (other players' pawns on a client) ---------------------------------------------------------------

/// UCharacterMovementComponent smoothing settings, written by its constructor rva=0x2f6cc10: NetworkSimulatedSmooth-
/// LocationTime 0.1 (+0x2b8, 0x142f6d0f4), NetworkSimulatedSmoothRotationTime 0.05 (+0x2bc; the
/// UAdvancedCharacterMovement ctor writes 0.05 again at 0x14144db49), NetworkMaxSmoothUpdateDistance 256 (+0x2d0,
/// 0x142f6d11c), NetworkNoSmoothUpdateDistance 384 (+0x2d4, 0x142f6d126), NetworkSmoothingMode Exponential (+0x16a = 2,
/// 0x142f6d130). Names CONFIRMED by the generated header (extract/match/gen UCharacterMovementComponent.h); the client
/// data copies them (FNetworkPredictionData_Client_Character ctor rva=0x2f6c6a0: MaxSmoothNetUpdateDist +0x118 = +0x2d0,
/// NoSmoothNetUpdateDist +0x11c = +0x2d4, SmoothNetUpdateTime +0x120 = +0x2b8 off a listen server).
/// BP_MordhauCharacter's ListenServerNetworkSimulatedSmoothLocationTime is 0.1 (package
/// Mordhau/Content/Mordhau/Blueprints/Characters/BP_MordhauCharacter); it applies to a listen server only.
#[derive(Clone, Copy, Debug)]
pub struct SmoothCfg {
    pub smooth_location_time: f32,
    pub max_smooth_update_distance: f32,
    pub no_smooth_update_distance: f32,
}

impl Default for SmoothCfg {
    fn default() -> Self {
        SmoothCfg { smooth_location_time: 0.1, max_smooth_update_distance: 256.0, no_smooth_update_distance: 384.0 }
    }
}

/// FRepMovement (AActor ReplicatedMovement) + ACharacter ReplicatedMovementMode, as a pawn replicates them to
/// simulated proxies, in the form the receiver reads them (quant.rs): Location at a pawn's LocationQuantizationLevel
/// RoundTwoDecimals (APawn::APawn rva=0x32ddce0, 0x1432dde76; 1/100 cm), LinearVelocity at RoundWholeNumber and
/// Rotation as ByteComponents (FRepMovement::FRepMovement rva=0x309c260 zeroes both levels; no Mordhau pawn class
/// changes them), through FRepMovement NetSerialize rva=0x35d9d00 / SerializeQuantizedVector rva=0x35d9de0. Only the
/// yaw byte is carried: a character's / horse's actor rotation has zero pitch and roll (PhysicsRotation keeps them
/// vertical), and a zero axis costs a single "zero" bit (FRotator::SerializeCompressed rva=0x18bf2e0).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RepMovement {
    pub location: [f32; 3],
    pub velocity: [f32; 3],
    pub yaw_byte: u8,
    pub mode: u8,
}

impl RepMovement {
    /// the replicated rotation's yaw (FRotator::DecompressAxisFromByte: Byte * 1.40625)
    pub fn yaw(&self) -> f32 {
        crate::quant::decompress_axis_from_byte(self.yaw_byte)
    }

    /// bits of FRepMovement's NetSerialize for this value: the 2 flag bits, the location and velocity packed vectors,
    /// the compressed rotator (bRepPhysics is false: no AngularVelocity) - what the send budget charges
    pub fn wire_bits(&self) -> u32 {
        let q = crate::quant::PAWN_REP_QUANTIZATION;
        let (_, lb) = crate::quant::quantize_vector(v3(self.location), q.location);
        let (_, vb) = crate::quant::quantize_vector(v3(self.velocity), q.velocity);
        2 + lb + vb + crate::quant::rotator_wire_bits([0, self.yaw_byte as u16, 0], q.rotation)
    }
}

/// the authority's ReplicatedMovement of a pawn (AActor::GatherCurrentMovement rva=0x2e2a1b0: the root component's
/// location, rotation and velocity), quantized as a pawn's FRepMovement serializes it (`RepMovement` docs)
pub fn rep_movement_of(m: &ExeMovement) -> RepMovement {
    let q = crate::quant::PAWN_REP_QUANTIZATION;
    let (l, _) = crate::quant::quantize_vector(m.location, q.location);
    let (vel, _) = crate::quant::quantize_vector(m.velocity, q.velocity);
    RepMovement { location: a3(l), velocity: a3(vel), yaw_byte: crate::quant::compress_axis_to_byte(m.yaw), mode: pack_mode(m.mode) }
}

/// A simulated proxy (ROLE_SimulatedProxy) on a client: the other pawn's own character movement component `m` (its
/// capsule, floor and movement mode on this machine's collision), driven by what the server replicates and, between
/// updates, by UCharacterMovementComponent::SimulateMovement; plus the mesh offset that hides corrections.
///
/// SimulatedTick rva=0x2f8bb80 (not playing root motion: SimulateMovement, then SmoothClientPosition) ->
/// UAdvancedCharacterMovement::SimulateMovement rva=0x14a1e00 (byte-matched src/Mordhau/Private/Components/
/// AdvancedCharacterMovement.cpp: bDoNotPredictMovement (+0xbc5) only lets a proxy simulate on the frame of a net
/// update; no constructor writes it (UAdvancedCharacterMovement 0x144da20, UMordhauMovementComponent 0x14af4f0,
/// UHorseMovementComponent 0x14ae880) and neither BP_MordhauCharacter nor BP_Horse sets it, so it is false) ->
/// UCharacterMovementComponent::SimulateMovement rva=0x2f8adf0, UE 4.26 structure (its calls and the landing compares
/// with MIN_FLOOR_DIST 1.9 / MAX_FLOOR_DIST 2.4 at 0x142f8b647 / 0x142f8b659 read off the disassembly; the rest of the
/// body UNCONFIRMED in detail):
///   - nothing until a ReplicatedMovement arrived (the zero Location / Rotation / LinearVelocity test, 0x142f8ae92..);
///   - on a net update: a changed ReplicatedMovementMode -> ApplyNetworkMovementMode rva=0x2f6ed80 (SetMovementMode);
///     else bJustTeleported / bForceNextFloorCheck -> UpdateFloorFromAdjustment (UAdvancedCharacterMovement 0x14a9720:
///     the engine's FindFloor while on the ground, bDoNotPredictMovement false);
///   - walking with a zero replicated velocity -> Velocity = 0 (bZeroReplicatedGroundVelocity);
///   - MoveSmooth rva=0x2f7c9d0 (walking: MoveAlongFloor; else a swept move and SlideAlongSurface; custom: a swept move);
///   - walking / falling: the floor (the step-down result, FindFloor when not moving up); no walkable floor -> fall
///     (NewFallVelocity) in MOVE_Falling; walkable and walking -> AdjustFloorHeight + SetBase; walkable and falling
///     within MIN_FLOOR_DIST -> landed (SetPostLandedPhysics: MOVE_Walking), else keep falling.
/// Not modelled: bSimGravityDisabled (a proxy replicated into penetration), UpdateProxyAcceleration (acceleration for
/// animation only), crouch on proxies (bIsCrouched replication), movement bases that move (ReplicatedBasedMovement:
/// see the module docs).
#[derive(Clone, Debug)]
pub struct ProxyMove {
    /// the proxy's movement component (authority false, not player controlled on this machine)
    pub m: ExeMovement,
    /// ClientData->MeshTranslationOffset
    pub offset: FVector,
    pub updates: u32,
    /// bNetworkUpdateReceived
    pub network_update_received: bool,
    /// bNetworkMovementModeChanged
    pub network_mode_changed: bool,
    /// the last ReplicatedMovement / ReplicatedMovementMode
    pub rep: RepMovement,
}

impl ProxyMove {
    /// `m` = a movement component of the pawn's class (ExeMovement::new / new_horse); the spawn's replication places it
    pub fn new(r: &RepMovement, mut m: ExeMovement) -> Self {
        m.authority = false;
        m.player_controlled = false;
        m.location = v3(r.location);
        m.velocity = v3(r.velocity);
        m.set_yaw(r.yaw());
        m.mode = unpack_mode(r.mode);
        m.force_next_floor_check = true;
        ProxyMove { m, offset: FVector::ZERO, updates: 0, network_update_received: true, network_mode_changed: false, rep: r.clone() }
    }

    /// convenience reads
    pub fn location(&self) -> FVector {
        self.m.location
    }
    pub fn velocity(&self) -> FVector {
        self.m.velocity
    }
    pub fn mode(&self) -> u8 {
        pack_mode(self.m.mode)
    }

    /// OnRep_ReplicatedMovement (AActor rva=0x2e34a40 / ACharacter rva=0x2f3dca0): APawn::PostNetReceiveVelocity
    /// (Velocity = the replicated one) and ACharacter::PostNetReceiveLocationAndRotation rva=0x2f3e9b0 (via
    /// AAdvancedCharacter's 0x1499cf0, which only forwards): bJustTeleported |= the location changed, then
    /// UAdvancedCharacterMovement::SmoothCorrection rva=0x14a2330 (Mordhau: rotation-smoothing timing around the
    /// engine's) -> UCharacterMovementComponent::SmoothCorrection rva=0x2f8e300 (Exponential, location part: the
    /// capsule goes to the new location, offset += OldLocation - NewLocation; farther than NetworkMaxSmoothUpdateDistance
    /// -> offset + MaxDist x safe normal; farther than NetworkNoSmoothUpdateDistance -> no smoothing (teleport));
    /// bNetworkUpdateReceived; ACharacter::PostNetReceive: bNetworkMovementModeChanged when ReplicatedMovementMode
    /// differs from the current packed mode. The location part of the engine's SmoothCorrection reads ClientData +0x118 / +0x11c (0x142f8e47d / 0x142f8e499)
    /// and writes MeshTranslationOffset +0xb0 (0x142f8e5a4 / 0x142f8e5d4); the rest of its body (timestamps, logging)
    /// does not move the mesh.
    pub fn on_rep(&mut self, r: &RepMovement, c: &SmoothCfg) {
        let old = self.m.location;
        let new = v3(r.location);
        self.m.velocity = v3(r.velocity);
        self.m.set_yaw(r.yaw());
        self.m.just_teleported |= old != new;
        self.m.location = new;
        let d = sub(old, new);
        let dsq = size_sq(d);
        if dsq > c.max_smooth_update_distance * c.max_smooth_update_distance {
            self.offset = if dsq > c.no_smooth_update_distance * c.no_smooth_update_distance {
                FVector::ZERO
            } else {
                let n = mul_v(d, 1.0 / dsq.sqrt());
                FVector { x: self.offset.x + n.x * c.max_smooth_update_distance, y: self.offset.y + n.y * c.max_smooth_update_distance, z: self.offset.z + n.z * c.max_smooth_update_distance }
            };
        } else {
            self.offset = FVector { x: self.offset.x + d.x, y: self.offset.y + d.y, z: self.offset.z + d.z };
        }
        self.network_mode_changed |= r.mode != pack_mode(self.m.mode);
        self.network_update_received = true;
        self.rep = r.clone();
        self.updates += 1;
    }

    /// UCharacterMovementComponent::SimulateMovement (type docs) on `world`, this machine's collision
    pub fn simulate_movement(&mut self, world: &dyn World, dt: f32) {
        let r = &self.rep;
        if r.location == [0.0; 3] && r.yaw_byte == 0 && r.velocity == [0.0; 3] {
            return; // replication not received yet (0x142f8ae92..0x142f8af1d)
        }
        let m = &mut self.m;
        if self.network_update_received {
            self.network_update_received = false;
            if self.network_mode_changed {
                // ApplyNetworkMovementMode: role != autonomous -> SetMovementMode(the unpacked mode)
                m.set_movement_mode(world, unpack_mode(r.mode));
                self.network_mode_changed = false;
            } else if m.just_teleported || m.force_next_floor_check {
                m.just_teleported = false;
                update_floor_from_adjustment(m, world);
            }
        } else if m.force_next_floor_check {
            update_floor_from_adjustment(m, world);
        }
        m.update_character_state_before_movement(world);
        if m.mode == Mode::None {
            return;
        }
        if m.is_moving_on_ground() && r.velocity == [0.0; 3] {
            m.velocity = FVector::ZERO; // bZeroReplicatedGroundVelocity
        }
        let mut sd = mh_character::exe::StepDownResult::default();
        move_smooth(m, world, m.velocity, dt, &mut sd);
        if m.is_moving_on_ground() || m.mode == Mode::Falling {
            if sd.computed_floor {
                m.current_floor = sd.floor;
            } else if m.velocity.z <= 0.0 {
                let loc = m.location;
                let zero = m.velocity == FVector::ZERO;
                let mut fl = m.current_floor;
                m.find_floor(world, loc, &mut fl, zero, None);
                m.current_floor = fl;
            } else {
                m.current_floor.clear();
            }
            if !m.current_floor.is_walkable_floor() {
                let g = FVector { x: 0.0, y: 0.0, z: m.gravity_z() };
                m.velocity = m.new_fall_velocity(m.velocity, g, dt);
                m.set_movement_mode(world, Mode::Falling);
            } else if m.is_moving_on_ground() {
                m.adjust_floor_height(world);
                m.base = m.current_floor.hit.component; // SetBase(CurrentFloor.HitResult.Component)
            } else if m.mode == Mode::Falling {
                if m.current_floor.floor_dist <= exe_cmc::MIN_FLOOR_DIST {
                    m.set_movement_mode(world, Mode::Walking); // SetPostLandedPhysics -> GroundMovementMode
                } else {
                    let g = FVector { x: 0.0, y: 0.0, z: m.gravity_z() };
                    m.velocity = m.new_fall_velocity(m.velocity, g, dt);
                    m.current_floor.clear();
                }
            }
        }
        m.update_character_state_after_movement(world);
        m.just_teleported = false;
        m.last_update_location = m.location;
    }

    /// one client frame of the proxy (SimulatedTick rva=0x2f8bb80): SimulateMovement, then the smoothing, read off the
    /// disassembly of UAdvancedCharacterMovement::SmoothClientPosition rva=0x14a1f70. Its own path runs when
    /// NetworkSmoothingMode (+0x16a) is Exponential (2: UCharacterMovementComponent ctor 0x142f6d130) and Mordhau's flag
    /// +0xb68 is set (UAdvancedCharacterMovement ctor 0x14144db2d: 1) - always for Mordhau's pawns:
    ///   SmoothLocationTime = ClientData SmoothNetUpdateTime (+0x120 = NetworkSimulatedSmoothLocationTime +0x2b8 = 0.1:
    ///   FNetworkPredictionData_Client_Character ctor rva=0x2f6c6a0, 0x142f6c86c), x0.5 (.rdata 0x143fe4e04) when
    ///   Velocity is zero (0x1414a1ffa..); DeltaTime < it -> MeshTranslationOffset x (1 - DeltaTime / it), else zero;
    ///   the rotation is slerped (not modelled: the proxy's yaw is the replicated one); when every offset component is
    ///   within 0.01 (.rdata 0x144014a94, 0x1414a2212..0x1414a2248) and the rotation is settled, bNetworkSmoothingComplete
    ///   (+0x1f1 bit 7) and the offset is zeroed (0x1414a22aa..0x1414a22c7); then SmoothClientPosition_UpdateVisuals.
    pub fn tick(&mut self, world: &dyn World, dt: f32, c: &SmoothCfg) {
        self.m.world_time += dt;
        self.simulate_movement(world, dt);
        let t = if self.m.velocity == FVector::ZERO { 0.5 * c.smooth_location_time } else { c.smooth_location_time };
        self.offset = if dt < t { mul_v(self.offset, 1.0 - dt / t) } else { FVector::ZERO };
        let k = f32::from_bits(0x3c23_d70a); // 0.01
        if self.offset.x.abs() <= k && self.offset.y.abs() <= k && self.offset.z.abs() <= k {
            self.offset = FVector::ZERO;
        }
    }

    /// where the mesh is drawn
    pub fn visual(&self) -> FVector {
        let l = self.m.location;
        FVector { x: l.x + self.offset.x, y: l.y + self.offset.y, z: l.z + self.offset.z }
    }
}

/// UAdvancedCharacterMovement::UpdateFloorFromAdjustment rva=0x14a9720 (bDoNotPredictMovement false ->
/// UCharacterMovementComponent::UpdateFloorFromAdjustment, UE 4.26: on the ground, FindFloor at the current location)
fn update_floor_from_adjustment(m: &mut ExeMovement, world: &dyn World) {
    if m.is_moving_on_ground() {
        let loc = m.location;
        let mut fl = m.current_floor;
        m.find_floor(world, loc, &mut fl, false, None);
        m.current_floor = fl;
    }
}

/// UCharacterMovementComponent::MoveSmooth rva=0x2f7c9d0 (UE 4.26 structure): custom -> a swept move by the velocity;
/// a zero delta does nothing; walking -> MoveAlongFloor; else a swept move and, on a blocking hit, SlideAlongSurface
/// (the StepUp branch is for flying only)
pub fn move_smooth(m: &mut ExeMovement, world: &dyn World, vel: FVector, dt: f32, sd: &mut mh_character::exe::StepDownResult) {
    let delta = mul_v(vel, dt);
    let mut hit = mh_character::HitResult::new(1.0);
    if m.mode == Mode::Custom {
        m.safe_move_updated_component(world, delta, &mut hit);
        return;
    }
    if delta == FVector::ZERO {
        return;
    }
    if m.is_moving_on_ground() {
        m.move_along_floor(world, vel, dt, sd);
    } else {
        m.safe_move_updated_component(world, delta, &mut hit);
        if hit.is_valid_blocking_hit() {
            let n = hit.normal;
            m.slide_along_surface(world, delta, 1.0 - hit.time, n, &mut hit);
        }
    }
}

// ---- server -------------------------------------------------------------------------------------------------------------

/// FNetworkPredictionData_Server_Character, the fields ProcessClientTimeStampForTimeDiscrepancy and
/// GetServerMoveDeltaTime read (offsets of this build)
#[derive(Clone, Debug, Default)]
pub struct ServerPrediction {
    /// +0x8 ServerTimeStamp (world time of the last ServerMove)
    pub server_time_stamp: f32,
    /// +0x60 CurrentClientTimeStamp
    pub current_client_time_stamp: f32,
    /// +0x74 ServerTimeStampLastServerMove (0: none yet)
    pub server_time_stamp_last_server_move: f32,
    /// +0x80 bit 0 bForceClientUpdate
    pub force_client_update: bool,
    /// +0x84 LifetimeRawTimeDiscrepancy
    pub lifetime_raw_time_discrepancy: f32,
    /// +0x88 TimeDiscrepancy
    pub time_discrepancy: f32,
    /// +0x8c bResolvingTimeDiscrepancy
    pub resolving: bool,
    /// +0x90 TimeDiscrepancyResolutionMoveDeltaOverride
    pub resolution_move_delta_override: f32,
    /// +0x94 TimeDiscrepancyAccumulatedClientDeltasSinceLastServerTick
    pub accumulated_client_deltas: f32,
    /// +0x98 WorldCreationTime
    pub world_creation_time: f32,
    /// AAdvancedCharacter LastPossessionTime (UAdvancedCharacterMovement::ProcessClientTimeStampForTimeDiscrepancy)
    pub last_possession_time: f32,
    /// PendingAdjustment, sent at the end of the server frame
    pub pending: Option<MoveResponse>,
    /// OnTimeDiscrepancyDetected calls (vtable +0xa08; UAdvancedCharacterMovement::OnTimeDiscrepancyDetected 0x14923b0
    /// only forwards to the engine's, which logs): (NewTimeDiscrepancy, LifetimeRaw, Lifetime, ClientError)
    pub discrepancies_detected: Vec<(f32, f32, f32, f32)>,
    pub moves: u32,
    pub old_moves: u32,
    pub stale_moves: u32,
    pub corrections_sent: u32,
    /// UCharacterMovementComponent bNetworkLargeClientCorrection (+0x1f2 bit 0)
    pub large_correction: bool,
    /// ServerLastClientGoodMoveAckTime / ServerLastClientAdjustmentTime (world time; 0 at construction)
    pub last_ack_time: f32,
    pub last_adjust_time: f32,
    /// answers SendClientAdjustment dropped by its time throttle (acks, corrections)
    pub acks_throttled: u32,
    pub corrections_throttled: u32,
    /// diagnostics: the last client errors found (time stamp, server location, client location, server / client mode)
    pub errors: VecDeque<(f32, [f32; 3], [f32; 3], u8, u8)>,
}

impl ServerPrediction {
    /// FNetworkPredictionData_Server_Character::GetServerMoveDeltaTime rva=0x2f7a800: bResolvingTimeDiscrepancy (+0x8c)
    /// -> TimeDiscrepancyResolutionMoveDeltaOverride (+0x90); else min(TimeStamp - CurrentClientTimeStamp,
    /// MaxMoveDeltaTime x TimeDilation)
    pub fn get_server_move_delta_time(&self, cfg: &NetMoveCfg, ts: f32, time_dilation: f32) -> f32 {
        if self.resolving {
            return self.resolution_move_delta_override;
        }
        let a = ts - self.current_client_time_stamp;
        let b = cfg.max_move_delta_time * time_dilation;
        if a < b {
            a
        } else {
            b
        }
    }

    /// UCharacterMovementComponent::ProcessClientTimeStampForTimeDiscrepancy rva=0x2f833a0, read instruction by
    /// instruction (field offsets above, AGameNetworkManager fields in NetMoveCfg), behind Mordhau's
    /// UAdvancedCharacterMovement override 0x149b1f0 (nothing during the 5 s after possession). `real_time` =
    /// RealTimeSeconds, `world_time` = TimeSeconds, CustomTimeDilation and GetActorTimeDilation 1 (no slomo here).
    pub fn process_client_time_stamp_for_time_discrepancy(&mut self, cfg: &NetMoveCfg, client_ts: f32, world_time: f32, real_time: f32) {
        if real_time < self.last_possession_time + cfg.possession_grace {
            return;
        }
        let has_moved = self.server_time_stamp_last_server_move != 0.0;
        if !(cfg.discrepancy_detection && has_moved) {
            return;
        }
        let custom_time_dilation = 1.0f32;
        let w = world_time;
        let client_delta = client_ts - self.current_client_time_stamp;
        let server_delta = (w - self.server_time_stamp) * custom_time_dilation;
        let client_error = client_delta - server_delta;
        let td_old = self.time_discrepancy;
        let raw = td_old + client_error;
        self.lifetime_raw_time_discrepancy += client_error;
        let drift = cfg.discrepancy_drift_allowance;
        let maxss = |a: f32, b: f32| if a > b { a } else { b }; // SSE maxss a, b
        let minss = |a: f32, b: f32| if a < b { a } else { b };
        let new_td;
        let eff;
        if drift > 0.0 && raw > 0.0 {
            let n = maxss(raw - drift * server_delta, 0.0);
            new_td = maxss(cfg.discrepancy_min_margin, n);
            eff = new_td / raw * client_error; // 0x142f834e2: no raw == 0 test on this path (raw > 0)
        } else {
            let n = if drift > 0.0 { minss(drift * server_delta + raw, 0.0) } else { raw };
            new_td = maxss(cfg.discrepancy_min_margin, n);
            eff = if raw != 0.0 { new_td / raw * client_error } else { client_error };
        }
        if self.resolving && td_old > 0.0 {
            self.resolving = true;
        } else {
            self.resolving = false;
            if new_td > cfg.discrepancy_max_margin {
                if cfg.discrepancy_resolution {
                    self.resolving = true;
                    self.time_discrepancy = new_td - eff;
                } else {
                    self.time_discrepancy = 0.0;
                }
                self.discrepancies_detected.push((new_td, self.lifetime_raw_time_discrepancy, w - self.world_creation_time, client_error));
            } else {
                self.time_discrepancy = new_td;
            }
        }
        if self.resolving {
            if cfg.discrepancy_force_corrections {
                self.force_client_update = true;
            }
            let since = (w - self.server_time_stamp) * custom_time_dilation;
            let first = since > 0.0;
            let time_dilation = 1.0f32;
            let mut bound = minss(time_dilation * cfg.max_move_delta_time, client_ts - self.current_client_time_stamp);
            if !first {
                self.accumulated_client_deltas += bound;
                bound += self.accumulated_client_deltas;
            } else {
                bound += self.accumulated_client_deltas;
                self.accumulated_client_deltas = 0.0;
            }
            bound = maxss(minss(bound, since), 0.0);
            let rate = if cfg.discrepancy_resolution_rate >= 0.0 { minss(cfg.discrepancy_resolution_rate, 1.0) } else { 0.0 };
            let td = self.time_discrepancy;
            let payback = minss(rate * bound, td);
            let after = maxss(bound - payback, MIN_TICK_TIME);
            self.resolution_move_delta_override = after;
            self.time_discrepancy = td - (bound - after);
        }
    }
}

/// UCharacterMovementComponent::ServerExceedsAllowablePositionError rva=0x2f86b10 (vtable +0x9b0): a client movement
/// mode other than PackNetworkMovementMode -> error; else LocDiff = server location - client location and
/// AGameNetworkManager::ExceedsAllowablePositionError rva=0x30d2a30: LocDiff.SizeSquared() > MAXPOSITIONERRORSQUARED.
/// ServerCheckClientError rva=0x2f86aa0 calls it unless bIgnoreClientMovementErrorChecksAndCorrection (+0x38c bit 0;
/// false here).
pub fn server_exceeds_allowable_position_error(cfg: &NetMoveCfg, m: &ExeMovement, client_loc: FVector, client_mode: u8) -> bool {
    server_exceeds_allowable_position_error_large(cfg, m, client_loc, client_mode).0
}

/// the same, with what it does to bNetworkLargeClientCorrection (disassembly of 0x2f86b10): a movement-mode mismatch
/// sets it (`or byte [rbx+0x1f2], 1` at 0x142f86b2b); a position error ORs in LocDiff.SizeSquared() >
/// NetworkLargeClientCorrectionDistance^2 (0x142f86bd2..0x142f86c0a). Returns (exceeds, large).
pub fn server_exceeds_allowable_position_error_large(cfg: &NetMoveCfg, m: &ExeMovement, client_loc: FVector, client_mode: u8) -> (bool, bool) {
    if client_mode != pack_mode(m.mode) {
        return (true, true);
    }
    let d = sub(m.location, client_loc);
    let dsq = size_sq(d);
    if dsq > cfg.max_position_error_squared {
        return (true, dsq > cfg.large_client_correction_distance * cfg.large_client_correction_distance);
    }
    (false, false)
}

/// UAdvancedCharacterMovement::ServerMove_PerformMovement 0x14a0230 (byte-matched in src/: HasValidData / IsActive,
/// GetServerMoveDeltaTime called once and dropped, then the engine's) -> UCharacterMovementComponent::
/// ServerMove_PerformMovement rva=0x2f88820: VerifyClientTimeStamp (IsClientTimeStampValid: newer than
/// CurrentClientTimeStamp; then ProcessClientTimeStampForTimeDiscrepancy), DeltaTime = GetServerMoveDeltaTime; > 0 ->
/// CurrentClientTimeStamp / ServerTimeStamp / ServerTimeStampLastServerMove, the control rotation, MoveAutonomous;
/// then ServerMoveHandleClientError rva=0x2f87570 (bForceClientUpdate or ServerCheckClientError -> PendingAdjustment
/// with the server's location / velocity / mode; else an acknowledged good move). Returns false for a stale move.
pub fn server_move_perform_movement(m: &mut ExeMovement, sp: &mut ServerPrediction, cfg: &NetMoveCfg, world: &dyn World, d: &ServerMoveData, world_time: f32, real_time: f32) -> bool {
    server_move_perform_movement_ex(m, sp, cfg, world, d, world_time, real_time, true)
}

/// the same with the NetworkMoveType test: only a NewMove (not the pending half of a ServerMoveDual) runs
/// ServerMoveHandleClientError
#[allow(clippy::too_many_arguments)]
pub fn server_move_perform_movement_ex(m: &mut ExeMovement, sp: &mut ServerPrediction, cfg: &NetMoveCfg, world: &dyn World, d: &ServerMoveData, world_time: f32, real_time: f32, check_error: bool) -> bool {
    let ts = d.time_stamp;
    if !(ts > sp.current_client_time_stamp) {
        sp.stale_moves += 1; // IsClientTimeStampValid: an old or duplicate move is ignored
        return false;
    }
    sp.process_client_time_stamp_for_time_discrepancy(cfg, ts, world_time, real_time);
    let dt = sp.get_server_move_delta_time(cfg, ts, 1.0);
    if dt > 0.0 {
        sp.current_client_time_stamp = ts;
        sp.server_time_stamp = world_time;
        sp.server_time_stamp_last_server_move = world_time;
        set_control_yaw(m, decompress_axis(d.yaw)); // SetControlRotation (a character's yaw follows it; a horse turns)
        move_autonomous(m, world, dt, d.flags, v3(d.accel));
        sp.moves += 1;
    }
    if !check_error {
        return true;
    }
    // ServerMoveHandleClientError: a newer pending adjustment is not overwritten
    if let Some(MoveResponse::AdjustPosition { time_stamp, .. } | MoveResponse::AckGoodMove { time_stamp }) = &sp.pending {
        if *time_stamp > ts {
            return true;
        }
    }
    // ServerMoveHandleClientError rva=0x2f87570: bNetworkLargeClientCorrection = bForceClientUpdate (0x142f8775d), then
    // ServerCheckClientError (vtable +0x9a8 = 0x2f86aa0 for every Mordhau movement class: no override)
    sp.large_correction = sp.force_client_update;
    let (exceeds, large) = server_exceeds_allowable_position_error_large(cfg, m, v3(d.location), d.mode);
    sp.large_correction |= large;
    if exceeds {
        if sp.errors.len() >= 32 {
            sp.errors.pop_front();
        }
        sp.errors.push_back((ts, a3(m.location), d.location, pack_mode(m.mode), d.mode));
    }
    if sp.force_client_update || exceeds {
        sp.pending = Some(MoveResponse::AdjustPosition { time_stamp: ts, location: a3(m.location), velocity: a3(m.velocity), mode: pack_mode(m.mode) });
    } else {
        sp.pending = Some(MoveResponse::AckGoodMove { time_stamp: ts });
    }
    sp.force_client_update = false;
    true
}

/// ACharacter::ServerMoveOld_Implementation rva=0x2f42230 -> UCharacterMovementComponent::ServerMoveOld: a re-sent
/// important move newer than CurrentClientTimeStamp: VerifyClientTimeStamp, MoveAutonomous(OldTimeStamp,
/// OldTimeStamp - CurrentClientTimeStamp, flags, accel), CurrentClientTimeStamp = OldTimeStamp (no error check)
pub fn server_move_old(m: &mut ExeMovement, sp: &mut ServerPrediction, cfg: &NetMoveCfg, world: &dyn World, d: &OldMoveData, world_time: f32, real_time: f32) -> bool {
    let ts = d.time_stamp;
    if !(ts > sp.current_client_time_stamp) {
        return false;
    }
    sp.process_client_time_stamp_for_time_discrepancy(cfg, ts, world_time, real_time);
    let dt = ts - sp.current_client_time_stamp;
    move_autonomous(m, world, dt, d.flags, v3(d.accel));
    sp.current_client_time_stamp = ts;
    sp.old_moves += 1;
    true
}

/// a received ServerMovePacket in the order the engine runs it: ServerMoveOld, then ServerMoveDual's pending move (no
/// error check: ACharacter::ServerMoveDual_Implementation rva=0x2f41f50), then the new move
pub fn server_move_packet(m: &mut ExeMovement, sp: &mut ServerPrediction, cfg: &NetMoveCfg, world: &dyn World, p: &ServerMovePacket, world_time: f32, real_time: f32) {
    if let Some(o) = &p.old {
        server_move_old(m, sp, cfg, world, o, world_time, real_time);
    }
    if let Some(d) = &p.pending {
        server_move_perform_movement_ex(m, sp, cfg, world, d, world_time, real_time, false);
    }
    server_move_perform_movement_ex(m, sp, cfg, world, &p.new, world_time, real_time, true);
}

/// UCharacterMovementComponent::SendClientAdjustment rva=0x2f85d00 (end of the server frame): the pending answer, if
/// any, then cleared
pub fn send_client_adjustment(sp: &mut ServerPrediction, now: f32, cfg: &NetMoveCfg) -> Option<MoveResponse> {
    // read off the disassembly (0x142f85d72..0x142f85e2f): an acknowledgement goes out only when TimeSeconds -
    // ServerLastClientGoodMoveAckTime > NetworkMinTimeBetweenClientAckGoodMoves; a correction only when TimeSeconds -
    // ServerLastClientAdjustmentTime > (bNetworkLargeClientCorrection ? min : max)(the two adjustment intervals);
    // either way the pending answer is cleared (a throttled one is dropped)
    let r = sp.pending.take()?;
    match r {
        MoveResponse::AckGoodMove { .. } => {
            if now - sp.last_ack_time > cfg.min_time_between_ack_good_moves {
                sp.last_ack_time = now;
                return Some(r);
            }
            sp.acks_throttled += 1;
            None
        }
        MoveResponse::AdjustPosition { .. } => {
            let (a, b) = (cfg.min_time_between_adjustments, cfg.min_time_between_adjustments_large);
            let threshold = if sp.large_correction { b.min(a) } else { b.max(a) };
            if now - sp.last_adjust_time > threshold {
                sp.last_adjust_time = now;
                sp.corrections_sent += 1;
                return Some(r);
            }
            sp.corrections_throttled += 1;
            None
        }
    }
}

/// The server's frame for a remotely controlled pawn, after the received moves: AMordhauCharacter::LODTick (the
/// authority parts), UMordhauMovementComponent::LODTick without its IsLocallyControlledOrUncontrolled block
/// (AAdvancedCharacter::IsLocallyControlledOrUncontrolled rva=0x1487940 is false for a remote player's pawn on the
/// server: the block 0x1414c4f0f..0x1414c79f7 - input, sprint flags, MovementModifier, dodge - is skipped, so the
/// MovementModifier is the one the last move's flags set) and its SprintTime tail (0x1414c79f7..0x1414c7a74, every
/// role), the ragdoll get-up and PostCharacterMovementTick.
pub fn server_remote_pawn_tick(m: &mut ExeMovement, dt: f32) {
    if m.horse.is_some() {
        // a remotely driven horse: AHorse::LODTick's DesiredGear only for an uncontrolled horse (it is controlled);
        // UHorseMovementComponent::LODTick's steering is the locally controlled block and its gear block runs on
        // "authority, locally controlled or uncontrolled" (none of them for a remote driver: the gear comes from the
        // moves' flags); UHorseMovementComponent::PostCharacterMovementTick rva=0xb93a60 is empty
        m.tick_ragdoll_get_up(dt);
        return;
    }
    m.character_lod_tick(dt);
    if (m.sprint_state as u8) < Sprint::Sprint as u8 {
        m.sprint_time = 0.0;
    } else {
        if m.sprint_state == Sprint::Rush && m.sprint_time < m.c.rush_sprint_time_start {
            m.sprint_time = m.c.rush_sprint_time_start;
        }
        m.sprint_time += dt;
        if m.world_time - m.creation_time < m.c.spawn_max_sprint_duration {
            m.sprint_time = if m.c.sprint_time_to_reach_max_sprint > m.sprint_time { m.c.sprint_time_to_reach_max_sprint } else { m.sprint_time };
        }
    }
    m.tick_ragdoll_get_up(dt);
    m.post_character_movement_tick(dt);
}

/// keep the exe_cmc import used (floor distance constant for hosts placing pawns)
pub const AVG_FLOOR_DIST: f32 = exe_cmc::AVG_FLOOR_DIST;

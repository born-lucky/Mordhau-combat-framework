//! UClimbingMotion (extract/native/decomp/UClimbingMotion.cpp), the combat side. The movement side (the authority
//! location moves of OnTick rva=0x1667790, MOVE_Custom / MOVE_Walking, CalculateLedgeOffsetAndNormal, turn caps) is
//! mh-character's exe_climb.rs, driven by the host (mh-sim) while this motion is current.
//!
//! - ctor rva=0x16457d0: ClimbRecoveryDuration 0.5, MovementRestriction 3, AuthorityMoveUpStartTime 0,
//!   AuthorityMoveLateralStartTime 0.25, AuthorityMoveLateralDuration 0.5, bCanAttack / bCanBlock false (the Slow*
//!   timings and Turncaps stay zero: UObject zero fill; no Blueprint subclass in the paks: UNCONFIRMED that the
//!   motion type 0x1b maps to the native class).
//! - AMordhauCharacter::RequestClimb rva=0x1564590: FNetMotion::Climbing rva=0x14b5380 (Offset = Target - root:
//!   PackFloat(X, -100..100), PackFloat(Y, -100..100), PackFloat(Z, 0..255), DynamicParam = bIsSlowClimb, type 0x1b),
//!   UMotionSystemComponent::AssignNetMotion -> ChangeMotion.
//! - OnBegin_Implementation rva=0x165e550: bIsSlowClimb = param byte == 1; ClimbOffset unpacked (byte * 1/255 clamped,
//!   * 200 - 100 / * 255) minus LedgeOffset (movement side); the slow timings swapped in; EndTime = StartTime +
//!   AuthorityMoveLateralStartTime + ClimbRecoveryDuration + AuthorityMoveLateralDuration; StopAnim(0.5) (animation
//!   side); SetTurnCaps(Turncaps) (movement side).
//! - OnLeave_Implementation rva=0x1665780: SetTurnCaps(-1, -1), MOVE_Walking (movement side).
//! - OnEnded: UMordhauMotion's (ChangeMotion(Idle)).

use super::motion::{MotionId, MotionKind};
use super::World;
use crate::data::{MotionBaseDef, MotionDef};
use crate::ue::FVector;
use std::rc::Rc;

#[derive(Clone, Debug, Default)]
pub struct ClimbingMotion {
    pub climb_recovery_duration: f64,           // +0xa8
    pub authority_move_up_start_time: f64,      // +0xb0
    pub authority_move_lateral_start_time: f64, // +0xb4
    pub authority_move_lateral_duration: f64,   // +0xb8
    pub slow_climb_recovery_duration: f64,      // +0xbc
    pub slow_authority_move_up_start_time: f64, // +0xc0
    pub slow_authority_move_lateral_start_time: f64, // +0xc4
    pub slow_authority_move_lateral_duration: f64,   // +0xc8
    pub turncaps: (f64, f64),                   // +0xcc
    pub b_is_slow_climb: bool,                  // +0xd4
    /// the net params (MotionParam0..2, DynamicParam) of FNetMotion::Climbing
    pub params: [u8; 4],
    /// ClimbOffset before the LedgeOffset subtraction (the movement side subtracts it)
    pub climb_offset: FVector,
}

impl ClimbingMotion {
    /// UClimbingMotion::UClimbingMotion rva=0x16457d0
    pub fn ctor() -> ClimbingMotion {
        ClimbingMotion {
            climb_recovery_duration: 0.5,
            authority_move_up_start_time: 0.0,
            authority_move_lateral_start_time: 0.25,
            authority_move_lateral_duration: 0.5,
            ..Default::default()
        }
    }
}

/// the native-only class defaults (UMordhauMotion ctor rva=0x1647f80 + UClimbingMotion ctor)
pub fn climbing_def() -> Rc<MotionDef> {
    Rc::new(MotionDef {
        base: MotionBaseDef {
            rec: "Climbing".into(),
            path: String::new(),
            native: "UClimbingMotion".into(),
            b_is_flinchable: true,
            b_can_attack: false,
            b_can_block: false,
            b_can_emote: false,
            b_blocks_regen: false,
            speed_factor: 1.0,
            backpedal_speed_factor: 1.0,
            shield_wall_speed_factor: 1.0,
            movement_restriction: 3,
        },
        ..Default::default()
    })
}

impl World {
    pub fn climb(&self, fi: usize, id: MotionId) -> Option<&ClimbingMotion> {
        if let MotionKind::Climbing(c) = &self.m(fi, id).k { Some(c) } else { None }
    }

    /// AMordhauCharacter::RequestClimb rva=0x1564590 with Offset = TargetLocation - RootComponent location: the
    /// climbing motion starts now (AssignNetMotion on the authority). Returns its id.
    pub fn request_climb(&mut self, fi: usize, offset: FVector, slow: bool) -> MotionId {
        let p0 = self.pack_float(offset.x as f64, -100.0, 100.0) as u8;
        let p1 = self.pack_float(offset.y as f64, -100.0, 100.0) as u8;
        let p2 = self.pack_float(offset.z as f64, 0.0, 255.0) as u8;
        if self.net.is_some() {
            // rust-net r5: on a networked machine RequestClimb is AssignNetMotion(FNetMotion::Climbing) (mh-net's
            // AssignNetMotion rules; HandleNetMotionUpdate creates the motion: begin_net_motion, net::CLIMBING). Offline
            // the shortcut below is unchanged.
            self.assign(fi, super::system::NetMotion::new(super::enums::net::CLIMBING, p0 as i64, p1 as i64, p2 as i64, slow as i64));
            return self.fighters[fi].motion.unwrap_or(MotionId(0));
        }
        let id = self.alloc_motion(fi, super::motion::Motion::new(climbing_def(), MotionKind::Climbing(Box::new(ClimbingMotion::ctor()))));
        if let MotionKind::Climbing(c) = &mut self.mm(fi, id).k {
            c.params = [p0, p1, p2, slow as u8];
        }
        self.change(fi, id);
        id
    }

    /// UClimbingMotion::OnBegin_Implementation rva=0x165e550, the combat fields
    pub(crate) fn climbing_on_begin(&mut self, fi: usize, id: MotionId) {
        let q = self.qf();
        let start = self.m(fi, id).start_time;
        let k = f32::from_bits(0x3b80_8081) as f64; // 0.003921569
        let unit = |b: u8| (b as f64 * k).clamp(0.0, 1.0);
        let end = {
            let MotionKind::Climbing(c) = &mut self.mm(fi, id).k else { return };
            c.b_is_slow_climb = c.params[3] == 1;
            c.climb_offset = FVector::new((unit(c.params[0]) * 200.0 - 100.0) as f32, (unit(c.params[1]) * 200.0 - 100.0) as f32, (unit(c.params[2]) * 255.0) as f32);
            if c.b_is_slow_climb {
                c.authority_move_lateral_start_time = c.slow_authority_move_lateral_start_time;
                c.authority_move_lateral_duration = c.slow_authority_move_lateral_duration;
                c.climb_recovery_duration = c.slow_climb_recovery_duration;
                c.authority_move_up_start_time = c.slow_authority_move_up_start_time;
            }
            q(q(q(start + c.authority_move_lateral_start_time) + c.climb_recovery_duration) + c.authority_move_lateral_duration)
        };
        self.mm(fi, id).end_time = end;
    }
}

//! Climbing in the Sim: mh-character's request side (JumpPressed -> TryClimbing -> AttemptClimb -> FindClimbSpot,
//! exe_climb.rs) feeds mordhau-core's UClimbingMotion (combat/climb.rs), whose movement side runs here every frame.
//!
//! - before the movement frame: AttemptClimb's host gates. motion_blocks_climb = the current motion is a
//!   UClimbingMotion, a UParryMotion, or a UAttackMotion with Stage != 2 (BP_MordhauCharacter AttemptClimb, as
//!   rust-character decoded it). equipment_prevents_climbing = false (UNCONFIRMED: bPreventsClimbing not in the records).
//! - after the frame: a climb_request -> World::request_climb(Target - root, slow) (AMordhauCharacter::RequestClimb
//!   rva=0x1564590) -> climb_on_begin (OnBegin rva=0x165e550's movement part: MOVE_Custom, ledge offset, timing).
//! - while the motion is current: climb_step (OnTick rva=0x1667790) toward ClimbTargetLocation.
//! - when it is no longer current: MOVE_Walking (OnLeave rva=0x1665780). SetTurnCaps(Turncaps) / (-1, -1) are not
//!   modelled (the Sim has no turn caps yet: UNCONFIRMED).

use crate::sim::Sim;
use mh_character::exe_climb::{climb_step, ClimbMotionData, ClimbState, ClimbTiming};
use mh_character::Mode;
use mordhau_core::combat::climb::ClimbingMotion;
use mordhau_core::combat::motion::MotionId;

#[derive(Clone, Debug)]
pub struct ClimbRun {
    pub id: MotionId,
    pub state: ClimbState,
    pub timing: ClimbTiming,
}

/// the UClimbingMotion class timings as mh-character's ClimbMotionData
fn motion_data(c: &ClimbingMotion) -> ClimbMotionData {
    ClimbMotionData {
        end_extra: c.climb_recovery_duration as f32,
        vertical_start: c.authority_move_up_start_time as f32,
        horizontal_start: c.authority_move_lateral_start_time as f32,
        horizontal_duration: c.authority_move_lateral_duration as f32,
        slow_end_extra: c.slow_climb_recovery_duration as f32,
        slow_vertical_start: c.slow_authority_move_up_start_time as f32,
        slow_horizontal_start: c.slow_authority_move_lateral_start_time as f32,
        slow_horizontal_duration: c.slow_authority_move_lateral_duration as f32,
        turn_cap_yaw: c.turncaps.0 as f32,
        turn_cap_pitch: c.turncaps.1 as f32,
    }
}

impl Sim {
    /// AttemptClimb's motion gate for fighter `fi` (before its movement frame)
    pub(crate) fn climb_gates(&mut self, fi: usize) {
        let blocks = self
            .combat
            .cur_m(fi)
            .map(|m| m.kind() == "Climbing" || m.is_parry() || m.attack().map(|a| a.stage != 2).unwrap_or(false))
            .unwrap_or(false);
        let m = &mut self.movers[fi];
        m.motion_blocks_climb = blocks;
        m.equipment_prevents_climbing = false;
    }

    /// after fighter `fi`'s movement frame: start a requested climb, step a running one, end a left one
    pub(crate) fn climb_post(&mut self, fi: usize) {
        while self.climbs.len() <= fi {
            self.climbs.push(None);
        }
        if let Some((target, slow)) = self.movers[fi].climb_request.take() {
            let root = self.movers[fi].location;
            let id = self.combat.request_climb(fi, target - root, slow);
            let c = self.combat.climb(fi, id).cloned().unwrap_or_else(ClimbingMotion::ctor);
            let data = motion_data(&ClimbingMotion::ctor());
            let start = self.combat.m(fi, id).start_time as f32;
            let b = self.movers[fi].climb_on_begin(self.collision.as_ref(), &data, c.params, start);
            self.climbs[fi] = Some(ClimbRun { id, state: ClimbState::default(), timing: b.timing });
        }
        let Some(run) = self.climbs[fi].as_mut() else { return };
        if self.combat.fighters[fi].motion == Some(run.id) {
            let target = self.movers[fi].climb_target_location;
            climb_step(&mut self.movers[fi], &mut run.state, &run.timing, target);
        } else {
            self.climbs[fi] = None;
            self.movers[fi].set_movement_mode(self.collision.as_ref(), Mode::Walking);
        }
    }
}

//! The movement side of climbing, for the combat side's UClimbingMotion (owner: rust-combat, mordhau-core). The motion
//! calls `climb_begin` from its OnBegin and `climb_step` from its OnTick; the motion's timing fields come from its class
//! data and net params (OnBegin_Implementation rva=0x165e550), which the caller fills in `ClimbTiming`.
//!
//! UClimbingMotion::OnTick_Implementation rva=0x1667790 moves the actor in two phases toward
//! AMordhauCharacter::ClimbTargetLocation (+0xdf8):
//!  - vertical: from StartTime + VerticalStart (+0xb0) to StartTime + HorizontalStart (+0xb4), Z goes from the Z it had
//!    when the phase began (latched once, +0x110 / +0x114) to Target.Z + 1;
//!  - horizontal: from StartTime + HorizontalStart to that + HorizontalDuration (+0xb8), X / Y go from the X / Y latched
//!    when the phase began (+0x108 / +0x10c, +0x115) to Target.X / Y;
//!  - the actor is set there (SetActorLocation, no sweep, no teleport) when that moves it more than 2 cm (squared
//!    distance > 4, .rdata 0x144014b50) or once TimeSeconds reaches the motion's EndTime (+0x50).
//!
//! It runs on the authority or the autonomous proxy only (0x1416677dd..0x1416677ed), and not in a vehicle
//! (CurrentVehicle +0x10c0). Not ported here: AMordhauCharacter::TryClimbing rva=0x16a61f0 (a
//! BlueprintImplementableEvent: BP_MordhauCharacter's graph picks the ledge) and CalculateLedgeOffsetAndNormal
//! rva=0x16a5290 (OnBegin's ledge offset).

use crate::exe::{ExeMovement, Mode};
use crate::ue::FVector;
use crate::uemath::v;
use crate::world::{HitResult, World};

/// The motion's timing (UClimbingMotion fields after OnBegin; the slow-climb variants are already swapped in)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClimbTiming {
    /// UMordhauMotion StartTime (+0x4c)
    pub start_time: f32,
    /// +0xb0: vertical phase start, seconds after StartTime
    pub vertical_start: f32,
    /// +0xb4: vertical phase end = horizontal phase start, seconds after StartTime
    pub horizontal_start: f32,
    /// +0xb8: horizontal phase length
    pub horizontal_duration: f32,
    /// +0x50 EndTime (OnBegin: StartTime + HorizontalStart + (+0xa8 + HorizontalDuration), 0x14165e7eb..0x14165e80c)
    pub end_time: f32,
}

/// The phase latches OnTick keeps in the motion
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClimbState {
    pub start_z: Option<f32>,
    pub start_xy: Option<(f32, f32)>,
}

/// UMordhauUtilityLibrary::GetNormalizedTime rva=0x1624620
pub fn normalized_time(start: f32, end: f32, t: f32) -> f32 {
    if t >= end {
        return 1.0;
    }
    if !(t > start) {
        return 0.0;
    }
    if start >= end {
        return 1.0;
    }
    (t - start) / (end - start)
}

/// OnBegin's movement part: SetMovementMode(MOVE_Custom) when the pawn's role is at least the autonomous proxy
/// (0x14165e811..0x14165e82d)
pub fn climb_begin(m: &mut ExeMovement, world: &dyn World) {
    m.set_movement_mode(world, Mode::Custom);
}

/// UClimbingMotion::OnTick_Implementation rva=0x1667790 (module docs). `target` = ClimbTargetLocation. Returns whether
/// the actor was moved.
pub fn climb_step(m: &mut ExeMovement, s: &mut ClimbState, tm: &ClimbTiming, target: FVector) -> bool {
    let now = m.world_time;
    let loc = m.location;
    let (mut nx, mut ny, mut nz) = (loc.x, loc.y, loc.z);
    let start = tm.start_time;
    let v0 = start + tm.vertical_start;
    if now >= v0 {
        let sz = *s.start_z.get_or_insert(loc.z);
        let a = normalized_time(v0, start + tm.horizontal_start, now);
        nz = ((target.z + 1.0) - sz) * a + sz;
    }
    let h0 = tm.horizontal_start + start;
    if now >= h0 {
        let (sx, sy) = *s.start_xy.get_or_insert((loc.x, loc.y));
        let a = normalized_time(h0, h0 + tm.horizontal_duration, now);
        nx = (target.x - sx) * a + sx;
        ny = (target.y - sy) * a + sy;
    }
    let (dy, dx, dz) = (ny - loc.y, nx - loc.x, nz - loc.z);
    let d = dy * dy + dx * dx + dz * dz;
    if d > 4.0 || now >= tm.end_time {
        m.location = v(nx, ny, nz); // AActor::SetActorLocation(bSweep false, Teleport None)
        return true;
    }
    false
}

// ---- the climb decision: AMordhauCharacter::JumpPressed / TryClimbing and BP_MordhauCharacter's graph ----------------
// BP_MordhauCharacter functions decoded from the package bytecode (scripts/kismet), cited by in-memory statement index.

/// first blocking hit of a capsule sweep (UKismetSystemLibrary::CapsuleTraceSingleByProfile, profile Pawn, simple
/// collision, no ignored actors besides self; ReturnValue = bBlockingHit)
fn capsule_trace(world: &dyn World, start: FVector, end: FVector, r: f32, hh: f32) -> Option<HitResult> {
    world.sweep_capsule(start, end, r, hh).into_iter().find(|h| h.blocking_hit)
}

fn vadd(a: FVector, b: FVector) -> FVector {
    v(a.x + b.x, a.y + b.y, a.z + b.z)
}
fn vsub(a: FVector, b: FVector) -> FVector {
    v(a.x - b.x, a.y - b.y, a.z - b.z)
}
fn vmul(a: FVector, k: f32) -> FVector {
    v(a.x * k, a.y * k, a.z * k)
}
/// UKismetMathLibrary::VSize -> FVector::Size
fn vsize(a: FVector) -> f32 {
    (a.x * a.x + a.y * a.y + a.z * a.z).sqrt()
}

impl ExeMovement {
    /// UCharacterMovementComponent::GetMaxJumpHeight rva=0x2f799b0: |GravityZ| > 1e-4 -> JumpZVelocity^2 * (-0.5 /
    /// GravityZ), else 0
    pub fn get_max_jump_height(&self) -> f32 {
        let g = self.gravity_z();
        if g.abs() > 1e-4 {
            let k = -0.5 / g;
            self.c.jump_z_velocity * self.c.jump_z_velocity * k
        } else {
            0.0
        }
    }

    /// BP_MordhauCharacter FindClimbSpot(UpwardsCast) for a player-controlled pawn (the MordhauAIController
    /// DesiredClimbTarget branch, statements 216..1024, is not ported)
    pub fn find_climb_spot(&self, world: &dyn World, upwards: f32) -> Option<FVector> {
        // 5..169: MinHeightToClimb = FMax(GetMaxJumpHeight() - 12, MaxStepHeight)
        let a = self.get_max_jump_height() - 12.0;
        let min_h = if a >= self.c.max_step_height { a } else { self.c.max_step_height };
        let (r, hh) = (self.capsule_radius(), self.half_height());
        let loc = self.location;
        let fwd = crate::uequat::actor_axes(self.yaw).1;
        // 1054..1384: cast up by UpwardsCast
        let up = vadd(loc, v(0.0, 0.0, upwards));
        let stage1 = match capsule_trace(world, loc, up, r, hh) {
            None => up, // 6524..6645
            Some(h) => {
                // 1561..1882: fails when the clearance left above the hit is under MinHeightToClimb
                if vsize(vsub(vadd(loc, v(0.0, 0.0, upwards)), h.location)) < min_h {
                    return None;
                }
                h.location
            }
        };
        // 1887..2937: cast forward two radii
        let ahead = vadd(stage1, vmul(fwd, r * 2.0));
        let stage2 = match capsule_trace(world, stage1, ahead, r, hh) {
            None => ahead, // 2726..2910
            Some(h) => {
                if vsize(vsub(stage1, h.location)) < r * 0.5 {
                    return None;
                }
                h.location
            }
        };
        // 3040..3398: cast down from Stage2 by UpwardsCast (OutHit_1); no floor -> fail
        let down = vadd(stage2, v(0.0, 0.0, upwards * -1.0));
        let hit1 = capsule_trace(world, stage2, down, r, hh)?;
        // 3446..4244: a half-radius capsule half a radius back, cast down the same way (OutHit_2); no hit -> fail
        let back = vmul(fwd, (r * 0.5) * -1.0);
        let hit2 = capsule_trace(world, vadd(stage2, back), vadd(down, back), r * 0.5, hh)?;
        let stage3 = hit1.location; // 4054..4217 (BreakHitResult of OutHit_1)
        // 4292..5313: actors that refuse climbing (host list)
        if let Some(c) = hit1.component {
            if self.climb_blocked_components.contains(&c) {
                return None;
            }
        }
        // 5318..6476: fail when the spot is lower than MinHeightToClimb above the pawn, the floor component refuses
        // step-up, either floor is steeper than WalkableFloorZ, or the two floors differ by more than 25 cm
        let wz = self.c.walkable_floor_z;
        let too_low = stage3.z - loc.z < min_h;
        let steep2 = hit2.impact_normal.z < wz;
        let no_step = !hit1.can_step_up;
        let steep1 = hit1.impact_normal.z < wz;
        let uneven = hit1.location.z - hit2.location.z > 25.0;
        if (too_low || no_step) || ((uneven || steep1) || steep2) {
            return None;
        }
        Some(stage3) // 6481..6519
    }

    /// BP_MordhauCharacter AttemptClimb: the gates (5..546), FindClimbSpot(200) then FindClimbSpot(100) (560..1036),
    /// ServerSetClimbLocation, the slow-climb choice (820..1777: falling faster than 500 cm/s; else a 0.75-radius
    /// capsule cast 200 cm down: no floor, or the target more than 125 cm above it), RequestClimb (1041)
    pub fn attempt_climb(&mut self, world: &dyn World) -> bool {
        if self.ragdoll_falling || !self.allow_climbing || self.equipment_prevents_climbing || self.motion_blocks_climb {
            return false;
        }
        let Some(target) = self.find_climb_spot(world, 200.0).or_else(|| self.find_climb_spot(world, 100.0)) else {
            return false;
        };
        self.climb_target_location = target; // 611 / 1009 (and ServerSetClimbLocation(FVector_NetQuantize))
        let slow = if self.velocity.z < -500.0 {
            true
        } else {
            let loc = self.location;
            match capsule_trace(world, loc, vadd(loc, v(0.0, 0.0, -200.0)), self.capsule_radius() * 0.75, self.half_height()) {
                Some(h) => vsub(target, h.location).z > 125.0,
                None => true,
            }
        };
        self.climb_request = Some((target, slow)); // AMordhauCharacter::execRequestClimb rva=0x16aa6f0
        true
    }

    /// BP_MordhauCharacter TryClimbing (the AMordhauCharacter::TryClimbing rva=0x16a61f0 event): (CanJump() ||
    /// (IsAirborne() && Velocity.Z > -900)) -> AttemptClimb
    pub fn try_climbing(&mut self, world: &dyn World) -> bool {
        let ok = self.can_jump() || (self.is_airborne() && self.velocity.z > -900.0);
        ok && self.attempt_climb(world)
    }

    /// the Jump action's edges: AMordhauCharacter::JumpPressed rva=0x154bff0 (bWantsClimb = false; TryClimbing, else
    /// bWantsClimb = true and ACharacter::Jump) / JumpReleased rva=0x154c030 (bWantsClimb = false; StopJumping)
    pub fn jump_input(&mut self, world: &dyn World, held: bool) {
        if held != self.jump_held {
            self.jump_held = held;
            self.wants_climb = false;
            if held {
                if !self.try_climbing(world) {
                    self.wants_climb = true;
                    self.jump();
                }
            } else {
                self.stop_jumping();
            }
        }
    }

    /// AMordhauCharacter::LODTick rva=0x154c390 0x14154c556..0x14154c584: bWantsClimb with no vehicle -> not airborne
    /// (IsAirborne, vcall +0x9e0) clears it; airborne retries TryClimbing each tick and clears it on success
    pub fn climb_retry(&mut self, world: &dyn World) {
        if self.wants_climb && (!self.is_airborne() || self.try_climbing(world)) {
            self.wants_climb = false;
        }
    }
}

// ---- OnBegin's movement-side pieces ----------------------------------------------------------------------------------

/// What UClimbingMotion::OnBegin_Implementation rva=0x165e550 derives for the motion (the combat side applies the
/// turn caps and the anim):
///  - `net_offset`: the replicated climb offset from the net motion params (bytes +0xc6 / +0xc7 / +0xc8 of the motion
///    system, each / 255 clamped to [0, 1]: X, Y -> * 200 - 100, Z -> * 255; 0x14165e64a..0x14165e700) minus the
///    ledge offset (0x14165e73c..0x14165e77f);
///  - `ledge_offset` / `ledge_normal`: BP_MordhauCharacter CalculateLedgeOffsetAndNormal (the
///    AMordhauCharacter::CalculateLedgeOffsetAndNormal rva=0x16a5290 event);
///  - `turn_caps`: SetTurnCaps(+0xcc, +0xd0) on the owner (vcall +0x9e8, AAdvancedCharacter::SetTurnCaps rva=0x14a15f0);
///  - `timing`: bIsSlowClimb (+0xd4 = net param byte +0xc9 == 1) swaps in the slow timings (+0xbc / +0xc0 / +0xc4 /
///    +0xc8 over +0xa8 / +0xb0 / +0xb4 / +0xb8), EndTime = StartTime + HorizontalStart + (+0xa8 + HorizontalDuration)
///    (0x14165e7eb..0x14165e80c);
///  - StopAnim(0.5) (0x14165e63b) and MOVE_Custom (`climb_begin`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClimbBegin {
    pub net_offset: FVector,
    pub ledge_offset: FVector,
    pub ledge_normal: FVector,
    pub turn_caps: (f32, f32),
    pub timing: ClimbTiming,
    pub stop_anim_blend_out: f32,
}

/// The motion's class timings (UClimbingMotion fields, read by OnBegin)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ClimbMotionData {
    pub end_extra: f32,                 // +0xa8 (added to HorizontalDuration for EndTime)
    pub vertical_start: f32,            // +0xb0
    pub horizontal_start: f32,          // +0xb4
    pub horizontal_duration: f32,       // +0xb8
    pub slow_end_extra: f32,            // +0xbc
    pub slow_vertical_start: f32,       // +0xc0
    pub slow_horizontal_start: f32,     // +0xc4
    pub slow_horizontal_duration: f32,  // +0xc8
    pub turn_cap_yaw: f32,              // +0xcc
    pub turn_cap_pitch: f32,            // +0xd0
}

impl ExeMovement {
    /// BP_MordhauCharacter CalculateLedgeOffsetAndNormal: a capsule cast from Location + (0, 0, 50) along the actor
    /// forward * 105; on a hit OutOffset = (HitLocation - Start) with Z 0, OutNormal = Normal((-ImpactNormal).XY0,
    /// 1e-4); else both zero (statements 0..1103)
    pub fn calculate_ledge_offset_and_normal(&self, world: &dyn World) -> (FVector, FVector) {
        let fwd = crate::uequat::actor_axes(self.yaw).1;
        let start = vadd(self.location, v(0.0, 0.0, 50.0));
        let end = vadd(start, vmul(fwd, 105.0));
        match capsule_trace(world, start, end, self.capsule_radius(), self.half_height()) {
            Some(h) => {
                let n = vmul(h.impact_normal, -1.0);
                let o = vsub(h.location, start);
                // UKismetMathLibrary::Normal(V, Tolerance) -> V.GetSafeNormal(Tolerance)
                let nn = crate::uemath::safe_normal_tol(v(n.x, n.y, 0.0), f32::from_bits(0x38d1_b717));
                (v(o.x, o.y, 0.0), nn)
            }
            None => (FVector::ZERO, FVector::ZERO),
        }
    }

    /// UClimbingMotion::OnBegin_Implementation rva=0x165e550's movement-side results (`ClimbBegin` docs); `params`
    /// are the net motion param bytes (+0xc6, +0xc7, +0xc8, +0xc9)
    pub fn climb_on_begin(&mut self, world: &dyn World, d: &ClimbMotionData, params: [u8; 4], start_time: f32) -> ClimbBegin {
        let k = f32::from_bits(0x3b80_8081); // 1 / 255 (.rdata 0x1440a4394)
        let unit = |b: u8| {
            let x = b as f32 * k;
            if x >= 0.0 {
                if x < 1.0 {
                    x
                } else {
                    1.0
                }
            } else {
                0.0
            }
        };
        let nx = unit(params[0]) * 200.0 - 100.0;
        let ny = unit(params[1]) * 200.0 - 100.0;
        let nz = unit(params[2]) * 255.0;
        let (lo, ln) = self.calculate_ledge_offset_and_normal(world);
        let slow = params[3] == 1;
        let (extra, vs, hs, hd) = if slow {
            (d.slow_end_extra, d.slow_vertical_start, d.slow_horizontal_start, d.slow_horizontal_duration)
        } else {
            (d.end_extra, d.vertical_start, d.horizontal_start, d.horizontal_duration)
        };
        let timing = ClimbTiming { start_time, vertical_start: vs, horizontal_start: hs, horizontal_duration: hd, end_time: start_time + hs + (extra + hd) };
        climb_begin(self, world);
        ClimbBegin {
            net_offset: v(nx - lo.x, ny - lo.y, nz - lo.z),
            ledge_offset: lo,
            ledge_normal: ln,
            turn_caps: (d.turn_cap_yaw, d.turn_cap_pitch),
            timing,
            stop_anim_blend_out: 0.5,
        }
    }
}

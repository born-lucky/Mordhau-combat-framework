//! Ladders: a vehicle mount. BP_Ladder (rust-pak's mh-world: LadderMount events, LadderInfo) spawns BP_LadderMover
//! (AMordhau1DVehicle, movement UOneDimensionalMovementComponent) on the ladder line and the character drives it
//! (BP_VehicleLadderMover, a UMordhauVehicleComponent with bIsLadder). The mover is an `ExeMovement` with
//! `ladder: Some(..)`; the engine velocity code is shared, the class overrides switch on it:
//!  - UOneDimensionalMovementComponent::GetMaxSpeed rva=0x14be440: walking -> the driver's
//!    UMordhauMovementComponent::GetSpeedFactor(0) (bUseDriverSpeedFactor, ctor rva=0x14afe70) * MaxWalkSpeed;
//!  - GetMaxAcceleration / GetMaxBrakingDeceleration: UCharacterMovementComponent's (ctor: 10000 / 10000);
//!  - ConstrainInputAcceleration rva=0x14b7ab0: the input as is (Z kept);
//!  - PhysWalking rva=0x14cfe50: CalcVelocity, then the move to FMath::ClosestPointOnLine rva=0x18a2a80 of the
//!    integrated location (no collision: the mover's capsule is NoCollision), velocity from the move, the step;
//!  - LODTick rva=0x14c85e0: the step target as forced input, characters ahead block the input;
//!  - OnMovementModeChanged rva=0x14cc410: anything but walking goes back to walking.
//! AMordhau1DVehicle::MoveForward rva=0x14ffd20 moves the target step (stepping outside the line requests a leave),
//! BP_LadderMover / BP_VehicleLadderMover (state/world_kismet dumps, cited `BP:Func@N`) the exit and the jump off.

use crate::exe::{ExeMovement, Mode};
use crate::records::CharacterRecords;
use crate::ue::FVector;
use crate::uemath::*;
use crate::uequat::*;
use crate::world::World;

/// The ladder's three component points (world cm) and facing, from mh-world's LadderInfo
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LadderInfo {
    pub start: FVector,
    pub end: FVector,
    pub exit: FVector,
    /// the ladder actor's yaw (its forward faces away from the wall)
    pub yaw: f32,
}

/// UOneDimensionalMovementComponent's own fields
#[derive(Clone, Debug, PartialEq)]
pub struct LadderState {
    pub info: LadderInfo,
    pub in_steps: bool,          // +0xc48 bIsMovementInSteps (BP_LadderMover true)
    pub use_driver_speed: bool,  // +0xc49 (ctor true)
    pub target_step: i32,        // +0xc4c
    pub step_size: f32,          // +0xc50 (BP 42.852)
    pub total_steps: i32,        // +0xc54
    pub initialized: bool,       // +0xc58 bHasInitializedLine
    pub current_step: i32,       // +0xc5c
    pub line_start: FVector,     // +0xc60
    pub line_end: FVector,       // +0xc6c
    pub direction: FVector,      // +0xc78
    /// the driver's UMordhauMovementComponent::GetSpeedFactor(0), the host's each frame
    pub driver_speed_factor: f32,
    pub has_driver: bool,
    pub last_requested_leave_time: f32, // AMordhau1DVehicle (RealTimeSeconds)
    pub leave_requests: u32,
    /// other characters near the line (root location, capsule radius / half height) for the LODTick block
    pub others: Vec<(FVector, f32, f32)>,
    pub secondary_turn: f32,     // UMordhauVehicleComponent SecondaryTurnValue (look sideways on the ladder)
}

/// FMath::ClosestPointOnLine rva=0x18a2a80 (segment): t = ((P - S) | D) / |D|^2 clamped to [0, 1]; D * t + S
pub fn closest_point_on_line(s: FVector, e: FVector, p: FVector) -> FVector {
    let d = v(e.x - s.x, e.y - s.y, e.z - s.z);
    let ps = v(p.x - s.x, p.y - s.y, p.z - s.z);
    let num = (ps.y * d.y + ps.x * d.x) + ps.z * d.z;
    let den = (d.y * d.y + d.x * d.x) + d.z * d.z;
    let t = num / den;
    let t = if t >= 0.0 { minss(t, 1.0) } else { 0.0 };
    v(d.x * t + s.x, d.y * t + s.y, d.z * t + s.z)
}

/// round(2x + 0.5) >> 1: FMath::RoundToInt via cvtss2si (round-to-even of 2x + 0.5, then halved)
fn round_to_int(x: f32) -> i32 {
    ((x + x + 0.5).round_ties_even() as i32) >> 1
}

impl ExeMovement {
    /// BP_Ladder:OnInteractionStart spawns BP_LadderMover at ClosestPointOnLine(LadderStart, LadderEnd, character
    /// location) with the ladder's rotation; BP_LadderMover:OnRep_Ladder@391 SetMovementLine(LadderStart, LadderEnd).
    /// The mover: BP_LadderMover CharMoveComp (MaxWalkSpeed 200, GravityScale 0, StepSize 42.852, bIsMovementInSteps)
    /// over the UOneDimensionalMovementComponent ctor (MaxAcceleration 10000, BrakingDecelerationWalking 10000).
    pub fn new_ladder_mover(base: &CharacterRecords, info: LadderInfo, character_location: FVector) -> ExeMovement {
        let p = closest_point_on_line(info.start, info.end, character_location);
        let mut m = ExeMovement::new(base, p);
        m.c.max_walk_speed = 200.0;
        m.c.max_acceleration = 10000.0;
        m.e.braking_deceleration_walking = 10000.0;
        m.c.gravity_scale = 0.0;
        m.c.ground_friction = 8.0; // UCharacterMovementComponent ctor 0x142f6cfff (BP_LadderMover keeps it)
        m.c.braking_friction_factor = 2.0; // ctor 0x142f6d1ba
        m.e.capsule_radius = 30.0;
        m.e.capsule_half_height = 96.0;
        m.capsule_half_height = 96.0;
        m.mode = Mode::Walking;
        m.set_yaw(info.yaw);
        m.ladder = Some(Box::new(LadderState {
            info,
            in_steps: true,
            use_driver_speed: true,
            target_step: 0,
            step_size: 42.852,
            total_steps: 0,
            initialized: false,
            current_step: 0,
            line_start: info.start,
            line_end: info.end,
            direction: FVector::ZERO,
            driver_speed_factor: 1.0,
            has_driver: false,
            last_requested_leave_time: f32::NEG_INFINITY,
            leave_requests: 0,
            others: Vec::new(),
            secondary_turn: 0.0,
        }));
        m.ladder_set_movement_line(info.start, info.end);
        m
    }

    fn l(&self) -> &LadderState {
        self.ladder.as_deref().expect("not a ladder mover")
    }
    fn lm(&mut self) -> &mut LadderState {
        self.ladder.as_deref_mut().expect("not a ladder mover")
    }

    /// UOneDimensionalMovementComponent::SetMovementLine rva=0x14d5a30
    pub fn ladder_set_movement_line(&mut self, s: FVector, e: FVector) {
        let loc = self.location;
        let l = self.lm();
        l.initialized = true;
        l.line_start = s;
        l.line_end = e;
        let d = v(e.x - s.x, e.y - s.y, e.z - s.z);
        let sq = (d.y * d.y + d.x * d.x) + d.z * d.z;
        l.direction = if sq == 1.0 {
            d
        } else if 1e-8 <= sq {
            let k = inv_sqrt(sq);
            v(d.x * k, d.y * k, d.z * k)
        } else {
            FVector::ZERO
        };
        if 0.0 < l.step_size {
            let inv = 1.0 / l.step_size;
            let len = ((d.y * d.y + d.x * d.x) + d.z * d.z).sqrt() * inv;
            l.total_steps = ((len + len - 0.5).round_ties_even() as i32) >> 1;
            let o = v(loc.x - s.x, loc.y - s.y, loc.z - s.z);
            let mut f = ((o.y * o.y + o.x * o.x) + o.z * o.z).sqrt() * inv;
            if f <= 0.0 {
                f = 0.0;
            }
            let n = round_to_int(f);
            l.target_step = n;
            l.current_step = n;
        }
    }

    /// UOneDimensionalMovementComponent::GetMaxSpeed rva=0x14be440
    pub fn ladder_max_speed(&self) -> f32 {
        let l = self.l();
        let f = if matches!(self.mode, Mode::Walking | Mode::NavWalking) && l.use_driver_speed && l.has_driver { l.driver_speed_factor } else { 1.0 };
        self.engine_max_speed() * f
    }

    /// AMordhau1DVehicle::MoveForward rva=0x14ffd20 (player controlled, bAddForwardAxisToMovementInput): in steps,
    /// stepping out of the line (forward at the top step, back at step 0) requests a leave (vcall +0xaa0, every 0.25 s
    /// real time), and the target step moves by the sign, clamped to [0, TotalSteps]
    pub fn ladder_move_forward(&mut self, value: f32) {
        if value == 0.0 {
            return;
        }
        let now = self.world_time;
        let l = self.lm();
        if l.in_steps && l.initialized {
            let out = if 0.0 < value { l.current_step == l.total_steps } else { l.current_step == 0 };
            if out && l.last_requested_leave_time + 0.25 < now {
                l.last_requested_leave_time = now;
                l.leave_requests += 1;
            }
        }
        if l.in_steps {
            let sign = if value <= 0.0 { if 0.0 <= value { 0 } else { -1 } } else { 1 };
            let n = l.current_step + sign;
            l.target_step = if -1 < n { if n < l.total_steps { n } else { l.total_steps } } else { 0 };
        }
    }

    /// UOneDimensionalMovementComponent::LODTick rva=0x14c85e0 (driven, line initialized)
    pub fn ladder_lod_tick(&mut self, dt: f32) {
        if !self.l().initialized || !self.l().has_driver {
            return;
        }
        let ms = self.ladder_max_speed();
        let loc = self.location;
        let l = self.l().clone();
        if l.in_steps && l.current_step != l.target_step && 0.0 < dt && 0.0 < self.c.max_walk_speed && 0.0 < l.step_size {
            let ts = l.target_step as f32 * l.step_size;
            let dz = ts * l.direction.z - (loc.z - l.line_start.z);
            let dy = ts * l.direction.y - (loc.y - l.line_start.y);
            let dx = ts * l.direction.x - (loc.x - l.line_start.x);
            let sq = (dx * dx + dy * dy) + dz * dz;
            let mut k = (1.0 / (ms * dt)) * sq.sqrt();
            if 1.0 <= k {
                k = 1.0;
            }
            let n = if sq == 1.0 {
                v(dx, dy, dz)
            } else if 1e-8 <= sq {
                let r = inv_sqrt(sq);
                v(r * dx, r * dy, r * dz)
            } else {
                FVector::ZERO
            };
            self.add_input_force(v(n.x * k, n.y * k, n.z * k));
        }
        // characters ahead block the input (the driver locally controlled)
        let input = self.control_input_vector;
        if input.x.abs() > 1e-4 || input.y.abs() > 1e-4 || input.z.abs() > 1e-4 {
            let t = if dt <= 0.02 { 0.02 } else { dt };
            let pred = v(input.x * t * ms + loc.x, input.y * t * ms + loc.y, input.z * t * ms + loc.z);
            let (r, hh) = (self.e.capsule_radius, self.capsule_half_height);
            for (o, orad, ohh) in l.others.iter() {
                let dpred = size_sq(sub(*o, pred));
                let dcur = size_sq(sub(*o, loc));
                // UPrimitiveComponent::OverlapComponent of their capsule with ours at the predicted spot (vertical
                // capsules: the axis segments within the summed radius)
                let overlap = {
                    let dxy = ((o.x - pred.x) * (o.x - pred.x) + (o.y - pred.y) * (o.y - pred.y)).sqrt();
                    let gap_z = (o.z - pred.z).abs() - ((ohh - orad) + (hh - r));
                    let dz = if gap_z > 0.0 { gap_z } else { 0.0 };
                    (dxy * dxy + dz * dz).sqrt() < orad + r
                };
                if dpred <= dcur && overlap {
                    self.add_input_force(v(-input.x, -input.y, -input.z));
                    break;
                }
            }
        }
    }

    /// UOneDimensionalMovementComponent::PhysWalking rva=0x14cfe50 (bCheatFlying clear)
    pub fn ladder_phys_walking(&mut self, dt: f32) {
        if !(dt >= 1e-6) || !self.l().initialized {
            return;
        }
        let f = self.c.ground_friction;
        let b = self.e.braking_deceleration_walking;
        self.calc_velocity(dt, f, b);
        let old = self.location;
        let target = v(dt * self.velocity.x + old.x, dt * self.velocity.y + old.y, dt * self.velocity.z + old.z);
        let l = self.l().clone();
        let pol = closest_point_on_line(l.line_start, l.line_end, target);
        // SafeMoveUpdatedComponent with a NoCollision capsule: the move happens as asked
        self.location = pol;
        let inv = 1.0 / dt;
        self.velocity = v((pol.x - old.x) * inv, (pol.y - old.y) * inv, (pol.z - old.z) * inv);
        if 0.0 < l.step_size {
            let o = v(pol.x - l.line_start.x, pol.y - l.line_start.y, pol.z - l.line_start.z);
            let mut s = ((o.x * o.x + o.y * o.y) + o.z * o.z).sqrt() * (1.0 / l.step_size);
            if s <= 0.0 {
                s = 0.0;
            }
            let n = round_to_int(s);
            if (n as f32 * l.step_size - s * l.step_size).abs() <= 2.5 {
                self.lm().current_step = n;
            }
        }
    }

    /// One frame of a driven ladder mover: MoveForward from the driver's forward axis, the LODTick, the movement
    /// (ControlledCharacterMove -> PerformMovement -> PhysWalking); returns whether the step changed
    /// (AMordhau1DVehicle::OnStepChanged from UOneDimensionalMovementComponent::TickComponent rva=0x14da130)
    pub fn ladder_frame(&mut self, world: &dyn World, dt: f32, fwd: f32) -> bool {
        self.world_time += dt;
        let step0 = self.l().current_step;
        self.ladder_move_forward(fwd);
        self.ladder_lod_tick(dt);
        let input = self.consume_input_vector();
        self.controlled_character_move(world, input, dt);
        self.l().current_step != step0
    }

    /// BP_VehicleLadderMover:GetExitTransform: within 50 cm of LadderEnd (@340) -> the LadderExit point (@388); else
    /// the base exit transform's location (`base_exit`, UMordhauVehicleComponent::GetExitTransform_Implementation: the
    /// driver's location or the Exit socket) - 50 along the ladder's forward (@724); then the dismount's
    /// FindTeleportSpot (UMordhauVehicleComponent::StopDriving rva=0x14d8270)
    pub fn ladder_exit(&self, world: &dyn World, base_exit: FVector, rider_radius: f32, rider_half_height: f32) -> FVector {
        let l = self.l();
        let d = sub(l.info.end, self.location);
        let place = if size(d) < 50.0 {
            l.info.exit
        } else {
            let f = rotator_vector(0.0, l.info.yaw);
            v(f.x * -50.0 + base_exit.x, f.y * -50.0 + base_exit.y, f.z * -50.0 + base_exit.z)
        };
        world.find_teleport_spot(place, rider_radius, rider_half_height).unwrap_or(place)
    }

    /// BP_LadderMover:PerformJumpoff: the launch the driver gets (LaunchCharacter, no overrides):
    /// (MapRangeClamped(MaxSpeedFalling, 300, 999, 1, 0.333) * 750, 0, 500) * (1 - 0.66 * (TotalSteps - CurrentStep <= 0)),
    /// rotated by the mover's yaw + SecondaryTurnValue (@380..@1163)
    pub fn ladder_jump_off_velocity(&self, driver_max_speed_falling: f32) -> FVector {
        let l = self.l();
        let at_top = if l.total_steps - l.current_step <= 0 { 1.0f32 } else { 0.0 };
        let k = 1.0 - at_top * f32::from_bits(0x3f28_f5c3);
        let pct = (driver_max_speed_falling - 300.0) / (999.0 - 300.0);
        let pct = if pct < 0.0 { 0.0 } else if pct > 1.0 { 1.0 } else { pct };
        let r = 1.0 + (f32::from_bits(0x3eaa_7efa) - 1.0) * pct;
        let lv = v(r * 750.0 * k, 0.0 * k, 500.0 * k);
        rotator_rotate_vector(0.0, self.yaw + l.secondary_turn, 0.0, lv)
    }

    /// BP_LadderMover:KnockOffDriver@2935: the driver trips and is knocked back -1750 along the mover's 2D forward
    /// (for the combat side's Knockback)
    pub fn ladder_knock_off_impulse(&self) -> FVector {
        let f = rotator_vector(0.0, self.yaw);
        let s = f.x * f.x + f.y * f.y;
        let (x, y) = if s > 1e-8 { let k = 1.0 / s.sqrt(); (f.x * k, f.y * k) } else { (0.0, 0.0) };
        v(x * -1750.0, y * -1750.0, 0.0)
    }
}

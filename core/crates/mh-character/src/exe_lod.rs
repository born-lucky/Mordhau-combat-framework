//! UMordhauMovementComponent::LODTick rva=0x14c4d60 (12001 bytes), ported from its disassembly with the Ghidra C as a
//! map: turn-sprint prevention, the input clamp and the avoidance "bubble" around enemies (with the input rotation
//! clamped to +-90 degrees), the chase bookkeeping, the sprint state, the dodge and its input override, SprintTime.
//! It runs for locally controlled (or uncontrolled) pawns, before the movement tick (UAdvancedCharacterMovement::
//! TickComponent vcall +0xb20 at 0x1414a41c7), and rewrites the pending input vector the movement then consumes.
//!
//! Horses in a character's bubble: their soft bubble (0x1414c6.. AHorse branch). Not ported: the gamepad branch (UMordhauUtilityLibrary::GetIsUsingController, the m.ControllerSprintStartAngle /
//! Min cvars), horses in the bubble pass (UHorseMovementComponent ellipse), the debug text.

use crate::exe::*;
use crate::ue::FVector;
use crate::uemath::*;
use crate::uequat::{actor_axes, atan2, calculate_direction, curve_eval, finterp_constant_to, quat_inverse, rotator_quaternion, unwind_degrees, Quat};

/// What LODTick reads of another pawn (TActorIterator<AAdvancedCharacter> order), and the chase timers it writes.
#[derive(Clone, Debug, Default)]
pub struct OtherPawn {
    pub alive: bool,
    /// an AMordhauCharacter
    pub mordhau_character: bool,
    /// its CharacterMovement is a UMordhauMovementComponent (a character's bubble needs it) / a
    /// UHorseMovementComponent (a horse's)
    pub mordhau_movement: bool,
    pub location: FVector,
    pub quat: Quat,
    pub forward: FVector,
    pub velocity: FVector,
    pub mesh_location: FVector,
    /// the "LowerBack" socket location of its mesh
    pub lower_back: FVector,
    /// AMordhauGameState::IsFriendly(self, other) (0x1414c5d0b)
    pub friendly: bool,
    /// CurrentVehicle != null (+0x10c0)
    pub in_vehicle: bool,
    /// held equipment bCanBeChasedFromFront (+0x68a of either hand)
    pub can_be_chased_from_front: bool,
    /// an AHorse (UHorseMovementComponent::LODTick takes its soft bubble: ellipse_* hold SoftBubbleEllipseLength /
    /// Radius / MaxHeight, centred at its location + SoftBubbleEllipseRelativeLocation rotated)
    pub is_horse: bool,
    pub soft_bubble_rel: FVector,
    /// its CharacterMovement is set (the horse LODTick tests it for characters)
    pub has_movement: bool,
    /// the horse's own driver (skipped by the horse's iteration)
    pub is_driver: bool,
    pub ellipse_length: f32,
    pub ellipse_radius: f32,
    pub ellipse_max_height_diff: f32,
    pub total_chased_time: f32,
    pub last_chased_time: f32,
}

impl OtherPawn {
    /// an enemy pawn with the BP_MordhauCharacter bubble, standing at `location` facing `yaw`
    pub fn enemy(m: &ExeMovement, location: FVector, yaw: f32) -> Self {
        let (q, f, _) = actor_axes(yaw);
        OtherPawn {
            alive: true,
            mordhau_character: true,
            mordhau_movement: true,
            location,
            quat: q,
            forward: f,
            velocity: FVector::ZERO,
            mesh_location: location,
            lower_back: location,
            friendly: false,
            in_vehicle: false,
            can_be_chased_from_front: false,
            is_horse: false,
            soft_bubble_rel: FVector::ZERO,
            has_movement: true,
            is_driver: false,
            ellipse_length: m.c.ellipse_bubble_length,
            ellipse_radius: m.c.ellipse_bubble_radius,
            ellipse_max_height_diff: m.c.ellipse_bubble_max_height_diff,
            total_chased_time: 0.0,
            last_chased_time: 0.0,
        }
    }
}

/// FQuat::RotateVector with the sums ordered (V + T * W) + (Q ^ T), as the chase and bubble sites add them
/// (0x1414c6059..0x1414c607f)
pub fn rotate_twv_pub(q: Quat, p: FVector) -> FVector {
    rotate_twv(q, p)
}

fn rotate_twv(q: Quat, p: FVector) -> FVector {
    let (qx, qy, qz, w) = (q[0], q[1], q[2], q[3]);
    let mut tx = qy * p.z - qz * p.y;
    let mut ty = qz * p.x - qx * p.z;
    let mut tz = qx * p.y - qy * p.x;
    tx = tx + tx;
    ty = ty + ty;
    tz = tz + tz;
    v((tx * w + p.x) + (qy * tz - qz * ty), (ty * w + p.y) + (qz * tx - qx * tz), (tz * w + p.z) + (qx * ty - qy * tx))
}

/// GetSafeNormal2D keeping the reference's (X, Y) and Z = 0 (the LODTick sites inline it with the == 1 shortcut)
fn safe_normal_2d_xy(x: f32, y: f32) -> (f32, f32) {
    let sq = y * y + x * x;
    if sq == 1.0 {
        return (x, y);
    }
    if sq < SMALL_NUMBER {
        return (0.0, 0.0);
    }
    let s = inv_sqrt(sq);
    (s * x, s * y)
}

impl ExeMovement {
    /// sets the actor rotation (FRotator(0, Yaw, 0)) and its quaternion
    pub fn set_yaw(&mut self, yaw: f32) {
        self.yaw = yaw;
        self.actor_quat = rotator_quaternion(0.0, yaw, 0.0);
    }

    /// UPawnMovementComponent::AddInputVector(V, bForce = true) -> ControlInputVector += V
    pub(crate) fn add_input_force(&mut self, d: FVector) {
        self.control_input_vector = add(self.control_input_vector, d);
    }

    /// ACharacter::StopJumping rva=0x2f448a0: bPressedJump = false; ResetJumpState
    pub fn stop_jumping(&mut self) {
        self.pressed_jump = false;
        self.was_jumping = false;
        self.jump_key_hold_time = 0.0;
        self.jump_force_time_remaining = 0.0;
        if !self.is_falling() {
            self.jump_current_count = 0;
        }
    }

    /// UMordhauMovementComponent::LODTick rva=0x14c4d60 (module docs)
    pub fn lod_tick(&mut self, dt: f32) {
        let now = self.world_time; // TimeSeconds (fStack_27c)
        let real = self.world_time; // RealTimeSeconds (fStack_284): no pause / dilation here
        self.is_chasing = false;
        let can_chase = !self.motion_disables_chase && !self.equipment_cannot_chase && !self.in_smoke_field; // uStack_288
        self.is_being_chased = false;
        // ---- player input block (IsLocallyControlledOrUncontrolled) ----
        let input0 = self.control_input_vector;
        let sq0 = input0.y * input0.y + input0.x * input0.x + input0.z * input0.z;
        let clamped_in = if sq0 <= 1.0 { input0 } else { mul(input0, inv_sqrt(sq0)) }; // fStack_280 / 2e4 / 2e0
        let sf0 = self.get_speed_factor(0.0); // fStack_270
        let mut push = FVector::ZERO; // fStack_2b8 / 2d4 / 2b4, later the scaled push
        if !self.ai_controlled {
            let mut input = input0;
            self.add_input_force(neg(input));
            let sq = input.y * input.y + input.x * input.x + input.z * input.z;
            if 1.0 < sq {
                input = mul(input, inv_sqrt(sq));
            }
            let yaw = self.yaw;
            if 5.0 < now - self.creation_time {
                // turn-sprint prevention (0x1414c5560..0x1414c5640)
                let mut d = yaw - self.turn_last_angle;
                if d > 180.0 {
                    d += -360.0;
                } else if d < -180.0 {
                    d += 360.0;
                }
                let maxa = self.c.turn_max_accumulated_angle;
                let c = d + self.turn_angle_counter;
                self.turn_angle_counter = if -maxa <= c {
                    if maxa <= c {
                        maxa
                    } else {
                        c
                    }
                } else {
                    -maxa
                };
                let decay = self.c.turn_decay_curve.as_ref().map_or(1.0, |k| curve_eval(k, self.turn_angle_counter.abs()));
                self.turn_angle_counter = finterp_constant_to(self.turn_angle_counter, 0.0, dt, decay);
                if let Some(k) = &self.c.turn_slowdown_curve {
                    let slow = curve_eval(k, self.turn_angle_counter.abs());
                    if slow < 1.0 {
                        input = if 1e-4 <= slow {
                            let s2 = input.y * input.y + input.x * input.x + input.z * input.z;
                            if slow * slow < s2 {
                                let k = inv_sqrt(s2) * slow;
                                v(input.x * k, input.y * k, k * input.z)
                            } else {
                                input
                            }
                        } else {
                            FVector::ZERO
                        };
                    }
                }
            }
            self.turn_last_angle = yaw;
            // predicted location for the bubble (0x1414c5848..0x1414c5b20)
            let loc = self.location;
            let falling = self.is_falling();
            let pred = if falling {
                v(dt * self.velocity.x, dt * self.velocity.y, dt * self.velocity.z)
            } else {
                let a = self.get_max_acceleration();
                let pz = a * input.z;
                let vz = self.velocity.z;
                let mut py = a * input.y + self.velocity.y;
                let mut px = a * input.x + self.velocity.x;
                let ms = self.get_max_speed();
                if 1e-4 <= ms {
                    let s2 = px * px + py * py;
                    if ms * ms < s2 {
                        let k = inv_sqrt(s2) * ms;
                        py = k * py;
                        px = k * px;
                    }
                } else {
                    py = 0.0;
                    px = 0.0;
                }
                let (vx, vy) = (self.velocity.x, self.velocity.y);
                let vs2 = vx * vx + vy * vy;
                let bd = self.e.braking_deceleration_walking;
                let mut r = vs2.sqrt() - bd * dt;
                if r <= 0.0 {
                    r = 0.0;
                }
                let (nx, ny, nz) = if vs2 == 1.0 {
                    if self.velocity.z == 0.0 {
                        (vx, vy, self.velocity.z)
                    } else {
                        (vx, vy, 0.0)
                    }
                } else if 1e-8 <= vs2 {
                    let s = inv_sqrt(vs2);
                    (s * vx, s * vy, 0.0)
                } else {
                    (0.0, 0.0, 0.0)
                };
                let by = r * ny;
                let bx = r * nx;
                let tstop = (bx * bx + by * by).sqrt() * (1.0 / bd);
                v((bx * 0.5) * tstop + dt * px, (by * 0.5) * tstop + dt * py, ((r * nz) * 0.5) * tstop + (pz + vz) * dt)
            };
            let pred = v(pred.x + loc.x, pred.y + loc.y, pred.z + loc.z);
            let no_input = !(input.x.abs() > 1e-4 || input.y.abs() > 1e-4 || input.z.abs() > 1e-4) && !falling;
            let my_mesh = self.mesh_location.unwrap_or(loc);
            let cam_loc = self.camera_location_1p.unwrap_or(loc);
            let cam_rot = self.camera_rotation_1p.unwrap_or((0.0, self.yaw, 0.0));
            let cam_inv = quat_inverse(rotator_quaternion(cam_rot.0, cam_rot.1, cam_rot.2));
            let bd_mine = self.e.braking_deceleration_walking;
            let mut others = std::mem::take(&mut self.others);
            for o in others.iter_mut() {
                if !o.alive || !(o.mordhau_character || o.is_horse) {
                    continue; // dead, neither an AMordhauCharacter nor an AHorse (0x1414c6..: the two IsA tests)
                }
                // an AHorse (0x1414c6..: its CharacterMovement IsA UHorseMovementComponent, else skipped): no chase
                // test, bThisCharacterForcesCorrection, the bubble its SoftBubbleEllipse Length / Radius / MaxHeight
                // centred at its location + its rotation * SoftBubbleEllipseRelativeLocation; the push direction
                // still uses its root location
                let mut centre = o.location;
                let forces;
                if o.is_horse && !o.mordhau_character {
                    if !o.mordhau_movement {
                        continue;
                    }
                    let rel = rotate_twv(o.quat, o.soft_bubble_rel);
                    centre = v(rel.x + o.location.x, rel.y + o.location.y, rel.z + o.location.z);
                    forces = true;
                } else if !o.in_vehicle {
                    if !o.friendly && can_chase {
                        // chase test (0x1414c5d95..0x1414c60f1)
                        let dx = o.mesh_location.x - my_mesh.x;
                        let dy = o.mesh_location.y - my_mesh.y;
                        let (nx, ny) = safe_normal_2d_xy(dx, dy);
                        let d = o.forward.y * ny + o.forward.x * nx + 0.0 * o.forward.z;
                        let d = if d >= -1.0 { minss(d, 1.0) } else { -1.0 };
                        let ang = d.acos() * RAD_TO_DEG;
                        let chasing_already = self.c.min_time_to_start_chasing <= o.total_chased_time;
                        let pass = o.can_be_chased_from_front
                            || if chasing_already { ang < self.c.max_angle_to_stop_chasing } else { !(ang >= self.c.max_angle_to_chase) };
                        if pass {
                            let rel = sub(o.lower_back, cam_loc);
                            let l = rotate_twv(cam_inv, rel);
                            let dist = if chasing_already { self.c.stop_chasing_max_distance } else { self.c.chasing_max_distance };
                            if l.x > 0.0 && !(l.x > dist.x) && !(l.y.abs() > dist.y) && !(l.z.abs() > dist.z) {
                                o.last_chased_time = real;
                            }
                        }
                    }
                    if o.last_chased_time + self.c.time_to_break_us_chasing >= real {
                        o.total_chased_time = dt + o.total_chased_time; // LODDeltaTime
                    } else {
                        o.total_chased_time = 0.0;
                    }
                    if o.total_chased_time >= self.c.min_time_to_start_chasing {
                        self.is_chasing = true;
                    }
                    if no_input {
                        continue;
                    }
                    forces = false;
                } else {
                    forces = true;
                }
                if !o.is_horse && (!o.mordhau_movement || (o.friendly && !o.in_vehicle)) {
                    continue;
                }
                // bubble (0x1414c6166..0x1414c6994)
                let (ll, rr, hh) = (o.ellipse_length, o.ellipse_radius, o.ellipse_max_height_diff);
                let ol = o.location;
                let t = (o.velocity.x * o.velocity.x + o.velocity.y * o.velocity.y).sqrt() * (1.0 / bd_mine);
                let sx = (o.velocity.x * 0.5) * t + centre.x;
                let sy = centre.y + (o.velocity.y * 0.5) * t;
                let dz = pred.z - centre.z;
                if dz.abs() > hh {
                    continue;
                }
                let ry = pred.y - sy;
                let rx = pred.x - sx;
                let qi = quat_inverse(o.quat);
                let (qx, qy, qz, w) = (qi[0], qi[1], qi[2], qi[3]);
                let mut tz = ry * qx - rx * qy;
                let mut ty = rx * qz - dz * qx;
                let mut tx = dz * qy - ry * qz;
                tz = tz + tz;
                ty = ty + ty;
                tx = tx + tx;
                let ly = (ry + w * ty) + (tx * qz - qx * tz);
                let lx = (tx * w + rx) + (qy * tz - qz * ty);
                let front = |lx: f32, ly: f32| -> FVector {
                    // circle around the front point (0x1414c665d)
                    let fx = ll - lx;
                    let d = ((-ly) * (-ly) + fx * fx).sqrt();
                    if d >= rr {
                        return FVector::ZERO;
                    }
                    let k = rr - d;
                    let ax = lx - ll;
                    let sq = ly * ly + ax * ax;
                    let (nx, ny) = if sq == 1.0 {
                        (ax, ly)
                    } else if sq < 1e-8 {
                        (0.0, 0.0)
                    } else {
                        let s = inv_sqrt(sq);
                        (ax * s, ly * s)
                    };
                    v(nx * k, ny * k, 0.0 * k)
                };
                let p = if lx > 0.0 {
                    if lx > ll || ly.abs() > rr {
                        front(lx, ly)
                    } else {
                        let rs = if ly > 0.0 { rr } else { -rr };
                        v(0.0, rs - ly, 0.0)
                    }
                } else {
                    let s2 = ly * ly + lx * lx;
                    let d = s2.sqrt();
                    if d >= rr {
                        front(lx, ly)
                    } else {
                        let k = rr - d;
                        let (nx, ny) = if s2 == 1.0 {
                            (lx, ly)
                        } else if s2 < 1e-8 {
                            (0.0, 0.0)
                        } else {
                            let s = inv_sqrt(s2);
                            (s * lx, s * ly)
                        };
                        v(k * nx, k * ny, k * 0.0)
                    }
                };
                if p.x.abs() > 1e-4 || p.y.abs() > 1e-4 || p.z.abs() > 1e-4 {
                    let world = rotate_twv(o.quat, p);
                    let mut dx = ol.x - pred.x;
                    let mut dy = ol.y - pred.y;
                    let s = dx * dx + dy * dy;
                    if 1e-8 < s {
                        let k = inv_sqrt(s);
                        dx = dx * k;
                        dy = dy * k;
                    }
                    let a = unwind_degrees((atan2(input.y, input.x) - atan2(dy, dx)) * RAD_TO_DEG);
                    if a.abs() < 10.0 && !falling && !forces {
                        input = FVector::ZERO;
                        self.was_dodge_canceled = true;
                    }
                    push = v(world.x + push.x, world.y + push.y, world.z + push.z);
                }
            }
            self.others = others;
            // push into input units, the clamped rotation (0x1414c69a6..0x1414c6d4a)
            let k = 1.0 / ((sf0 * self.c.max_walk_speed) * dt);
            push = v(k * push.x, k * push.y, k * push.z);
            let mut nx = push.x + input.x;
            let mut ny = push.y + input.y;
            let mut nz = push.z + input.z;
            let s = nx * nx + ny * ny + nz * nz;
            if 1.0 < s {
                let q = inv_sqrt(s);
                nx = q * nx;
                ny = q * ny;
                nz = q * nz;
            }
            let mut a = (atan2(ny, nx) - atan2(input.y, input.x)) * RAD_TO_DEG;
            while 180.0 < a {
                a += -360.0;
            }
            while a < -180.0 {
                a += 360.0;
            }
            let a = if -90.0 <= a {
                if 90.0 <= a {
                    90.0
                } else {
                    a
                }
            } else {
                -90.0
            };
            if self.is_falling() {
                input = v(nx, ny, nz);
            } else {
                let q = rotator_quaternion(0.0, a, 0.0);
                input = crate::uequat::quat_rotate(q, input);
            }
            self.add_input_force(input);
        }
        // being chased (0x1414c6d8b..)
        if real <= self.last_chased_time + self.c.time_to_break_us_being_chased {
            self.total_chased_time = dt + self.total_chased_time;
        } else {
            self.total_chased_time = 0.0;
        }
        if self.c.min_time_to_start_being_chased <= self.total_chased_time {
            self.is_being_chased = true;
        }
        // ---- sprint flags and state (0x1414c6ddb..0x1414c720a) ----
        let speed = size(self.velocity);
        let r = self.get_movement_restriction();
        let k = sf0;
        self.only_partial_sprint = r == restriction::PARTIAL_SPRINT;
        if r != restriction::PARTIAL_SPRINT && speed + RDATA_SPRINT_SPEED_SLACK < k * self.c.max_walk_speed * self.c.partial_sprint_modifier {
            self.only_partial_sprint = true;
        }
        self.sprint_allowed = if r == restriction::WALK { false } else { !(speed + RDATA_SPRINT_SPEED_SLACK < k * self.c.max_walk_speed) };
        let rush = match self.rush_perk {
            Some((boost, last_kill)) => now < boost + last_kill,
            None => false,
        };
        let dir = if self.ai_controlled { safe_normal(self.velocity) } else { safe_normal(self.control_input_vector) };
        if dir.x == 0.0 && dir.y == 0.0 && dir.z == 0.0 {
            self.sprint_state = Sprint::Forward;
        } else {
            let (_, fwd, _) = actor_axes(self.yaw);
            let d = fwd.y * dir.y + fwd.x * dir.x + fwd.z * dir.z;
            let c = if d < -1.0 { -1.0 } else { minss(d, 1.0) };
            let a = unwind_degrees(c.acos() * RAD_TO_DEG).abs();
            self.sprint_state = Sprint::Forward;
            if a > RDATA_FORWARD_CONE_DEG {
                self.wants_supersprint = false;
                self.sprint_state = if a > RDATA_BACKPEDAL_DEG { Sprint::Backpedal } else { Sprint::Sideways };
            } else if self.wants_supersprint {
                self.sprint_state = Sprint::Super;
            } else if (rush || self.wants_sprint) && self.sprint_allowed {
                self.sprint_state = if self.only_partial_sprint {
                    Sprint::Partial
                } else if self.is_chasing {
                    Sprint::Chase
                } else if rush {
                    Sprint::Rush
                } else {
                    Sprint::Sprint
                };
            }
        }
        // ---- dodge (0x1414c720a..0x1414c79f1) ----
        let can_dodge_now = !(r == restriction::NO_MOVEMENT
            || self.equipment_disables_dodge
            || self.motion_disables_dodge
            || self.is_falling()
            || self.is_crouched);
        if self.pressed_jump && self.c.can_dodge {
            let qi = quat_inverse(self.actor_quat);
            let l = crate::uequat::quat_rotate(qi, clamped_in);
            if l.x <= 0.01 && (l.x.abs() > 1e-4 || l.y.abs() > 1e-4 || l.z.abs() > 1e-4) {
                self.stop_jumping(); // vcall +0x7a0 (0x1414c7376)
                if can_dodge_now
                    && self.c.dodge_stamina_cost <= self.stamina
                    && self.c.dodge_duration + self.last_dodge_time + self.c.dodge_cooldown < now
                {
                    self.last_dodge_time = now;
                    self.was_dodge_canceled = false;
                    let s = clamped_in.y * clamped_in.y + clamped_in.x * clamped_in.x + clamped_in.z * clamped_in.z;
                    let dd = if s == 1.0 {
                        clamped_in
                    } else if 1e-8 <= s {
                        let q = inv_sqrt(s);
                        v(q * clamped_in.x, q * clamped_in.y, q * clamped_in.z)
                    } else {
                        FVector::ZERO
                    };
                    self.dodge_direction = dd;
                    self.dodge_direction_local = crate::uequat::quat_rotate(qi, dd);
                    let deg = atan2(dd.y, dd.x) * RAD_TO_DEG;
                    let mut a = if 0.0 <= deg { deg } else { deg + 360.0 };
                    if a < 0.0 {
                        a = 0.0;
                    }
                    if 360.0 <= a {
                        a = 360.0;
                    }
                    let x = a * f32::from_bits(0x3f35_5556); // 0.7083334 (256 / 360 rounded as the exe stores it)
                    let i = (((x + x + 0.5) as f64).round_ties_even() as i64) >> 1; // RoundToInt: cvtss2si(2x + 0.5) >> 1
                    self.dodges.push(i.clamp(0, 255) as u8); // AMordhauCharacter::ServerRequestDodge rva=0x16a5cd0
                }
            }
        }
        if now <= self.c.dodge_duration + self.last_dodge_time && !self.was_dodge_canceled {
            let a1 = calculate_direction(self.dodge_direction_local, 0.0, 0.0, 0.0);
            let a2 = calculate_direction(self.dodge_direction, 0.0, self.yaw, 0.0);
            let mut d = a2 - a1;
            if d > 180.0 {
                d += -360.0;
            } else if d < -180.0 {
                d += 360.0;
            }
            if can_dodge_now && d.abs() <= 45.0 {
                self.sprint_state = Sprint::Super;
                let wv = crate::uequat::quat_rotate(self.actor_quat, self.dodge_direction_local);
                let mut w = v(wv.x + push.x, wv.y + push.y, wv.z + push.z);
                let s = w.x * w.x + w.y * w.y + w.z * w.z;
                if 1.0 < s {
                    let q = inv_sqrt(s);
                    w = v(q * w.x, q * w.y, w.z * q);
                }
                let p = self.control_input_vector;
                self.add_input_force(v(w.x - p.x, w.y - p.y, w.z - p.z));
            } else {
                self.was_dodge_canceled = true;
            }
        }
        // ---- SprintTime (0x1414c79f7..0x1414c7a74) ----
        if self.sprint_state < Sprint::Sprint {
            self.sprint_time = 0.0;
        } else {
            if self.sprint_state == Sprint::Rush && self.sprint_time < self.c.rush_sprint_time_start {
                self.sprint_time = self.c.rush_sprint_time_start;
            }
            self.sprint_time = dt + self.sprint_time;
            if now - self.creation_time < self.c.spawn_max_sprint_duration {
                self.sprint_time = maxss(self.c.sprint_time_to_reach_max_sprint, self.sprint_time);
            }
        }
    }
}

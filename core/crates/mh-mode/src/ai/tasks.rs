//! The behavior-tree tasks the bot trees use (godot/game/ai/bt/*.gd). Results are EBTNodeResult values, the same
//! integers the game's tasks return in the Ghidra C: Succeeded 0, Failed 1, Aborted 2, InProgress 3 (e.g.
//! UBTTask_MeleeDefend::AbortTask rva=0x1456e80 returns 2). execute = ExecuteTask, tick = TickTask (only while latent),
//! abort = AbortTask.
//!   MeleeAttack      UBTTask_MeleeAttack::PerformOffensiveEvaluation rva=0x1495d00 (ExecuteTask rva=0x147fe60 jumps to
//!                    it; TickTask rva=0x14a5120); argument values Ghidra dropped are from the disassembly ("disasm <addr>")
//!   MeleeDefend      UBTTask_MeleeDefend::PerformDefensiveEvaluation rva=0x14940a0, ExecuteTask rva=0x147fe70, TickTask
//!                    rva=0x14a5160, AbortTask rva=0x1456e80, StopCrouching rva=0x14a3460
//!   FallForFeint     UBTTask_FallForFeint::PerformFeintEvaluation rva=0x1495720 (ExecuteTask rva=0x147fe50, TickTask
//!                    rva=0x14a50e0)
//!   SwitchEquipment  UBTTask_SwitchEquipment::PerformSwitchingTask rva=0x1497830, ctor rva=0x144dcb0
//!   VoiceOrEmote     UBTTask_VoiceOrEmote::ExecuteTask rva=0x147fed0, ctor rva=0x144dd00
//!   Wait             UBTTask_Wait::ExecuteTask rva=0x38a5d40 / TickTask rva=0x38bb8c0 (bt.rs R10)
//!   BackOff          BTTask_BackOff_C bytecode (Mordhau/Content/Mordhau/AI/Tasks/BTTask_BackOff; literals ai_backoff_*)
//!   FindRandomLocation / FindUnstuckSpot / MoveToDestination   BTTask_*_C bytecode (literals ai_find_*, ai_unstuck_*,
//!                    ai_moveto_*); navigation queries through the host (team navigation filter not modelled:
//!                    UNCONFIRMED)
//! Any other task (Blueprint without bytecode, ranged) fails (UNCONFIRMED stand-in).

use super::body::{enums as E, BotBody};
use super::bt::{BbVal, TaskParams, ABORTED, FAILED, IN_PROGRESS, SUCCEEDED};
use super::controller::{BotController, BotEvent, Ctx, Facing, PathStatus};
use crate::consts::bot as K;
use mordhau_core::ue::{clampf, f32r, maxf, FVector};

/// the safe normal UE's FVector::GetSafeNormal inlines in every task (rsqrt + two Newton steps; exact 1.0 kept,
/// squared length < 1e-8 -> zero vector): _DAT_143fe4e0c = 1.0, _DAT_144014a88 = 1e-8 (bt_task.gd safe_normal; Godot
/// arithmetic: f32 squared length, `v / sqrt(l2)` divides by the f32 of the f64 root)
pub fn safe_normal(v: FVector) -> FVector {
    let l2 = v.length_squared();
    if l2 == 1.0 {
        return v;
    }
    if (l2 as f64) < K::SAFE_NORMAL_MIN_SQ {
        return FVector::ZERO;
    }
    let s = (l2 as f64).sqrt() as f32;
    FVector::new(v.x / s, v.y / s, v.z / s)
}

/// FRotator(0, yaw, 0).RotateVector: rotation about Z, +yaw turns X toward Y (UE convention)
pub fn rotate_yaw(v: FVector, yaw_deg: f64) -> FVector {
    let r = yaw_deg.to_radians();
    let (x, y) = (v.x as f64, v.y as f64);
    FVector::new((x * r.cos() - y * r.sin()) as f32, (x * r.sin() + y * r.cos()) as f32, v.z)
}

/// AActor::GetActorForwardVector of a body in the active numeric model: the exe's quaternion path
/// (mordhau_core::ue::ue_actor_forward_yaw: FRotator::Quaternion then GetForwardVector rva=0x2fa7ca0) or the
/// reference's (cos, sin) of the f64 yaw
pub fn body_forward(cx: &Ctx, b: usize) -> FVector {
    if cx.exe {
        mordhau_core::ue::ue_actor_forward_yaw(cx.bodies[b].yaw as f32)
    } else {
        cx.bodies[b].forward()
    }
}

/// FRotator(0, yaw, 0).RotateVector in the active numeric model (exe: rva=0x18bd9b0, mordhau_core::ue::ue_rotate_yaw)
pub fn rotate_yaw_m(cx: &Ctx, v: FVector, yaw: f64) -> FVector {
    if cx.exe {
        mordhau_core::ue::ue_rotate_yaw(v, yaw as f32)
    } else {
        rotate_yaw(v, yaw)
    }
}

/// CalculateAngle2D in the active numeric model: the exe's instruction sequence (ue_math, f32 yaw) or the GDScript
/// reference's (f64 trig)
pub fn angle_2d(cx: &Ctx, d: FVector, yaw: f64) -> f64 {
    if cx.exe {
        crate::ue_math::calculate_angle_2d([d.x, d.y, d.z], yaw as f32) as f64
    } else {
        calculate_angle_2d(d, yaw)
    }
}

fn v2_len_sq(x: f32, y: f32) -> f32 {
    x * x + y * y
}

/// UMordhauUtilityLibrary::CalculateAngle2D rva=0x1616170: signed angle (degrees) of `d` from the forward of
/// FRotator(0, yaw, 0), positive toward its right axis; all |components| <= 1e-4 (_DAT_144022350) -> 0.
/// acos * _DAT_1442713d0 (57.29578)
pub fn calculate_angle_2d(d: FVector, yaw: f64) -> f64 {
    let e = K::ANGLE2D_ZERO_COMPONENT;
    if (d.x.abs() as f64) <= e && (d.y.abs() as f64) <= e && (d.z.abs() as f64) <= e {
        return 0.0;
    }
    let r = yaw.to_radians();
    let fwd = (r.cos() as f32, r.sin() as f32);
    let right = (-r.sin() as f32, r.cos() as f32);
    let (mut dx, mut dy) = (d.x, d.y);
    let l2 = v2_len_sq(dx, dy);
    if l2 != 1.0 {
        if (l2 as f64) >= K::ANGLE2D_MIN_SQ {
            // Godot Vector2::normalized: x /= sqrt(l), y /= sqrt(l) (f32)
            let l = l2.sqrt();
            dx /= l;
            dy /= l;
        } else {
            dx = 0.0;
            dy = 0.0;
        }
    }
    let dot = (fwd.0 * dx + fwd.1 * dy) as f64;
    let a = clampf(dot, -1.0, 1.0).acos() * K::ANGLE2D_RAD_TO_DEG;
    if ((right.0 * dx + right.1 * dy) as f64) < 0.0 {
        -a
    } else {
        a
    }
}

/// FMath::FindDeltaAngleDegrees rva=0x1480350 (UE 4.26 UnrealMathUtility.h: A2 - A1 wrapped to [-180, 180])
pub fn find_delta_angle(a1: f64, a2: f64) -> f64 {
    let mut d = a2 - a1;
    if d > 180.0 {
        d -= 360.0;
    } else if d < -180.0 {
        d += 360.0;
    }
    d
}

/// vertical capsule centred at `c`: segment of half length (half_height - radius) on Z, inflated by radius. Closest
/// distance between segment ab and the vertical axis segment, by clamped closest-point parameters (bt_melee_defend.gd;
/// UNCONFIRMED: collision margins of the engine's UPrimitiveComponent::LineTraceComponent)
pub fn segment_hits_capsule(a: FVector, b: FVector, c: FVector, half_height: f64, radius: f64) -> bool {
    let h = maxf(half_height - radius, 0.0);
    let p0 = c - FVector::new(0.0, 0.0, h as f32);
    let d1 = b - a;
    let d2 = FVector::new(0.0, 0.0, (2.0 * h) as f32);
    let r = a - p0;
    let aa = d1.dot(d1) as f64;
    let ee = d2.dot(d2) as f64;
    let ff = d2.dot(r) as f64;
    let mut s = 0.0;
    let mut t = 0.0;
    if aa <= 1e-12 && ee <= 1e-12 {
        return (r.length() as f64) <= radius;
    }
    if aa <= 1e-12 {
        t = clampf(ff / ee, 0.0, 1.0);
    } else {
        let cc = d1.dot(r) as f64;
        if ee <= 1e-12 {
            s = clampf(-cc / aa, 0.0, 1.0);
        } else {
            let bb = d1.dot(d2) as f64;
            let den = aa * ee - bb * bb;
            s = if den != 0.0 { clampf((bb * ff - cc * ee) / den, 0.0, 1.0) } else { 0.0 };
            t = (bb * s + ff) / ee;
            if t < 0.0 {
                t = 0.0;
                s = clampf(-cc / aa, 0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = clampf((bb - cc) / aa, 0.0, 1.0);
            }
        }
    }
    let p = a + d1.scale(s);
    let q = p0 + d2.scale(t);
    ((p - q).length() as f64) <= radius
}

/// a task node instance: its name, class and parameters + the task's own state
#[derive(Clone, Debug)]
pub struct TaskNode {
    pub name: String,
    pub type_: String,
    pub params: TaskParams,
    pub kind: Task,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Task {
    MeleeDefend,
    MeleeAttack,
    FallForFeint,
    SwitchEquipment,
    VoiceOrEmote,
    FindHordeTask,
    Wait { remaining: f64 },
    BackOff(BackOff),
    FindRandomLocation,
    FindUnstuckSpot,
    MoveToDestination(MoveTo),
    Unported { result: i32 },
}

/// BTTask_BackOff_C variables (none in the CDO: 0 / false)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BackOff {
    pub requires_initialization: bool,
    pub original_angle: f64,
    pub right_offset: f64,
    pub random_side: f64,
    pub distance: f64,
}

/// BTTask_MoveToDestination_C variables
#[derive(Clone, Debug, PartialEq)]
pub struct MoveTo {
    pub go_to_actor: bool,
    pub last_actor_location: FVector,
    pub move_target_location: FVector,
    pub current_movement_time: f64,
    pub current_movement_distance: f64,
    /// MordhauGameState bAllowHealthRegen (the host sets the mode's value)
    pub allow_health_regen: bool,
}

impl TaskNode {
    /// ported native tasks by UE class name (BtTree.native_task)
    pub fn new(name: &str, type_: &str, params: TaskParams) -> TaskNode {
        let kind = match type_ {
            "BTTask_MeleeDefend" => Task::MeleeDefend,
            "BTTask_MeleeAttack" => Task::MeleeAttack,
            "BTTask_FallForFeint" => Task::FallForFeint,
            "BTTask_SwitchEquipment" => Task::SwitchEquipment,
            "BTTask_VoiceOrEmote" => Task::VoiceOrEmote,
            "BTTask_Wait" => Task::Wait { remaining: 0.0 },
            "BTTask_BackOff_C" => Task::BackOff(BackOff::default()),
            "BTTask_FindRandomLocation_C" => Task::FindRandomLocation,
            "BTTask_FindUnstuckSpot_C" => Task::FindUnstuckSpot,
            "BTTask_FindHordeTask_C" => Task::FindHordeTask,
            "BTTask_MoveToDestination_C" => Task::MoveToDestination(MoveTo {
                go_to_actor: false,
                last_actor_location: FVector::ZERO,
                move_target_location: FVector::ZERO,
                current_movement_time: 0.0,
                current_movement_distance: 0.0,
                allow_health_regen: true,
            }),
            _ => Task::Unported { result: FAILED },
        };
        TaskNode { name: name.to_string(), type_: type_.to_string(), params, kind }
    }

    pub fn execute(&mut self, c: &mut BotController, cx: &mut Ctx) -> i32 {
        let p = &self.params;
        match &mut self.kind {
            Task::MeleeDefend => defend_execute(c, cx),
            Task::MeleeAttack => attack_perform(c, cx),
            Task::FallForFeint => feint_perform(c, cx),
            Task::SwitchEquipment => switch_perform(c, cx, p),
            Task::VoiceOrEmote => voice_execute(c, cx, p),
            Task::Wait { remaining } => {
                // R10
                let (w, dv) = (p.wait_time, p.random_deviation);
                let lo = maxf(w - dv, 0.0);
                let q = cx.q;
                *remaining = q(q(q(cx.rng.rand() as f64 * K::BT_WAIT_RAND_SCALE) * q(q(w + dv) - lo)) + lo);
                IN_PROGRESS
            }
            Task::BackOff(b) => b.execute(c, cx),
            Task::FindRandomLocation => find_random_location(c, cx, p),
            Task::FindUnstuckSpot => find_unstuck_spot(c, cx, p),
            Task::FindHordeTask => find_horde_task(c, cx, p),
            Task::MoveToDestination(m) => m.execute(c, cx, p),
            Task::Unported { result } => {
                let r = *result;
                let s = format!("{} not ported -> {}", self.type_, super::bt::RESULT_NAMES[r as usize]);
                c.note(cx, "BT", s, "");
                r
            }
        }
    }

    pub fn tick(&mut self, c: &mut BotController, cx: &mut Ctx, dt: f64) -> i32 {
        let p = &self.params;
        match &mut self.kind {
            Task::MeleeDefend => defend_execute(c, cx),
            Task::MeleeAttack => attack_perform(c, cx),
            Task::FallForFeint => feint_perform(c, cx),
            Task::SwitchEquipment => switch_perform(c, cx, p),
            Task::Wait { remaining } => {
                *remaining = (cx.q)(*remaining - dt);
                if *remaining <= 0.0 {
                    SUCCEEDED
                } else {
                    IN_PROGRESS
                }
            }
            Task::BackOff(b) => b.tick(c, cx, dt),
            Task::MoveToDestination(m) => m.tick(c, cx, p, dt),
            _ => IN_PROGRESS,
        }
    }

    pub fn abort(&mut self, c: &mut BotController, cx: &mut Ctx) -> i32 {
        match &mut self.kind {
            Task::MeleeDefend => {
                stop_crouching_task(c, cx);
                ABORTED
            }
            Task::MoveToDestination(_) => {
                c.stop_movement();
                ABORTED
            }
            _ => ABORTED,
        }
    }
}

// ==== UBTTask_MeleeAttack ==========================================================================================
const ATTACK_FN: &str = "UBTTask_MeleeAttack::PerformOffensiveEvaluation rva=0x1495d00";

fn attack_perform(c: &mut BotController, cx: &mut Ctx) -> i32 {
    let me_i = c.body;
    if !cx.bodies[me_i].has_sys {
        return FAILED;
    }
    let now = cx.now;
    let my_loc = cx.bodies[me_i].location;
    let Some(w) = cx.bodies[me_i].weapon.clone() else { return FAILED }; // RightHandEquipment (+0x11f8)
    if !w.b_can_attack {
        return FAILED; // AMordhauEquipment +0xcb9 bCanAttack
    }
    let en = c.closest_enemy(cx); // AMordhauAIController::GetClosestEnemy rva=0x14fa570
    let en = match en {
        Some(e) if !cx.bodies[e].is_dead => e,
        _ => {
            c.note(cx, "MeleeAttack", "no live enemy -> RequestFeint, Failed".into(), ATTACK_FN);
            c.request_feint(cx);
            return FAILED;
        }
    };
    let enemy_attacking = cx.bodies[en].pawn.motion.is_attack; // UAttackMotion or URangedDrawMotion -> fStackX_10
    let ally = c.closest_ally(cx); // GetClosestAlly rva=0x14fa3b0
    let rnd = c.get_motion_based_random(cx); // controller +0x32c
    let own = cx.bodies[me_i].pawn.motion.clone();
    let my_atk = if own.is_attack { Some(own.clone()) } else { None };
    let (me, enb) = (&cx.bodies[me_i], &cx.bodies[en]);
    let q = cx.q;
    let time_left = match &my_atk {
        Some(a) => q(a.release_end - now),
        None => K::ATTACK_DEFAULT_TIME_LEFT, // 0.9
    };
    let reach = q(q(q(q(me.scaled_radius()) + q(enb.scaled_radius())) + q(q(me.weapon_length * K::WEAPON_LENGTH_TO_CM) * me.mesh_scale_x))
        + q(q(me.threat_speed() - q(enb.threat_speed() * K::ATTACK_ENEMY_SPEED_FACTOR)) * time_left)); // 15, 0.6
    let my_parry = if own.is_parry { Some(own.clone()) } else { None };
    let dist = (enb.location - my_loc).length() as f64; // AActor::GetDistanceTo rva=0x2e2c0b0
    let jitter = maxf(q(q(q(me.weapon_length - K::ATTACK_JITTER_LENGTH_OFFSET) * K::ATTACK_JITTER_SCALE) * me.mesh_scale_x), 0.0);
    let move_dist = q(q(jitter * rnd) + K::ATTACK_CIRCLE_BASE); // (Length - 2) * 10 * scale * rnd + 140
    let own_desc = match &my_atk {
        Some(a) => format!("attack {}", E::stage_name(a.stage)),
        None => own.kind.clone(),
    };
    c.note(cx, "MeleeAttack", format!("dist {:.1} range {:.1} (own {})", dist, reach, own_desc), ATTACK_FN);
    if q(reach + K::ATTACK_OUT_OF_RANGE_MARGIN) < dist
        && my_atk.as_ref().map(|a| a.stage == E::WINDUP).unwrap_or(false)
        && c.profile.p.will_out_of_range_feint
    {
        if c.facing_mode == Facing::Location {
            // GetCurrentFacingMode rva=0x14fafc0
            return FAILED;
        }
        c.note(cx, "MeleeAttack", "out of range in windup, bWillOutOfRangeFeint -> RequestFeint, Failed".into(), ATTACK_FN);
        c.request_feint(cx);
        return FAILED;
    }
    let en_loc = cx.bodies[en].location;
    let bone_h = cx.bodies[en].facing_bone_height;
    if q(reach + K::ATTACK_ENGAGE_MARGIN) <= dist {
        c.start_facing_movement(0.0); // 200
    } else {
        c.start_facing_bone(en, bone_h, [0.0, 0.0], now);
    }
    let may_move = c.get_move_status() == PathStatus::Idle || ((en_loc.z - my_loc.z).abs() as f64) < K::ATTACK_MOVE_MAX_ZDIFF; // 50
    c.start_sprinting(cx);
    let dir = safe_normal(my_loc - en_loc);
    if may_move {
        circle(c, cx, en, ally, dir, rnd, move_dist);
    }
    let stage = my_atk.as_ref().map(|a| a.stage).unwrap_or(-1);
    let mut choose = false;
    if my_atk.is_none() || stage == E::RECOVERY {
        if reach < dist {
            return if q(reach + K::ATTACK_ENGAGE_MARGIN) < dist { FAILED } else { IN_PROGRESS };
        }
        let in_riposte = my_parry
            .as_ref()
            .map(|p| !(q(q(p.riposte_window_start + p.riposte_window_base) + p.riposte_window_extra) < now))
            .unwrap_or(false);
        if !in_riposte {
            let until = q(own.start_time + c.profile.p.attack_hesitance_random); // Motion StartTime + AttackHesitanceRandom
            if now < until {
                c.note(cx, "MeleeAttack", format!("hesitating until {:.4} -> Failed", until), ATTACK_FN);
                if !may_move {
                    return FAILED;
                }
                // side step: enemy + FRotator(0, 45 - 90 * rnd, 0).RotateVector(dir * 400), acceptance -1
                // (PerformOffensiveEvaluation disasm 0x141496a28: xmm13 = -1 from 0x14149648c)
                // exe @0x1414969a1-@0x141496a0a: dir * 400 (mulss), 45 - rnd * 90 (f32)
                let yaw = if cx.exe {
                    (K::ATTACK_SIDE_STEP_YAW as f32 - rnd as f32 * K::ATTACK_SIDE_STEP_YAW_RANGE as f32) as f64
                } else {
                    K::ATTACK_SIDE_STEP_YAW - rnd * K::ATTACK_SIDE_STEP_YAW_RANGE
                };
                let off = rotate_yaw_m(cx, dir.scale(K::ATTACK_SIDE_STEP_DIST), yaw);
                c.move_to_location(en_loc + off, K::DEFAULT_ACCEPTANCE, now);
                return FAILED;
            }
        } else if !c.profile.p.will_riposte {
            return IN_PROGRESS;
        }
        choose = true;
    } else {
        if stage == E::RELEASE {
            if reach < dist {
                return IN_PROGRESS;
            }
            if !c.profile.p.will_combo {
                return FAILED;
            }
        }
        // LAB_1414968d1
        if stage != E::WINDUP || (c.profile.p.will_morph && !enemy_attacking) {
            choose = true;
        }
    }
    if choose {
        choose_and_request(c, cx, &w, dist);
    }
    // LAB after the request (rva 0x1496b7d...): look at the attack the bot is in now
    let atk = {
        let m = &cx.bodies[me_i].pawn.motion;
        if m.is_attack {
            Some(m.clone())
        } else {
            None
        }
    };
    let chambered = atk.as_ref().map(|a| a.stage == E::WINDUP && a.has_chambered).unwrap_or(false); // +0x10f4
    c.get_motion_based_random(cx);
    let Some(atk) = atk else { return FAILED };
    match atk.stage {
        E::RECOVERY => return FAILED,
        E::WINDUP => {
            let feint_at = q(q(atk.windup_end - K::ATTACK_FEINT_LEAD) - c.profile.p.feint_timing_random); // 0.15
            if cx.bodies[me_i].stamina_byte() - cx.bodies[en].stamina_byte() > 0
                && !chambered
                && c.profile.p.will_feint
                && !enemy_attacking
                && feint_at < now
            {
                c.note(cx, "MeleeAttack", format!("feint: more stamina, bWillFeint, now > {:.4} -> RequestFeint", feint_at), ATTACK_FN);
                c.request_feint(cx);
                return if cx.bodies[me_i].pawn.motion.id != atk.id { FAILED } else { IN_PROGRESS };
            }
        }
        E::RELEASE => {
            let accel = c.profile.p.will_accel || enemy_attacking;
            if ((c.profile.p.will_drag && !enemy_attacking) || accel) && E::is_strike(atk.mv) {
                let mut pitch = atk.angle_target * K::ATTACK_DRAG_PITCH_SCALE; // * -60
                let mut yawo = K::ATTACK_DRAG_YAW_BASE - atk.angle_target.abs() * K::ATTACK_DRAG_YAW_PER_ANGLE; // 60 - |a| * 30
                if E::is_left(atk.mv) {
                    yawo *= K::ATTACK_FLIP;
                }
                if accel {
                    yawo *= K::ATTACK_FLIP;
                    pitch *= K::ATTACK_FLIP;
                }
                let s = format!("{}: facing offset ({:.2}, {:.2})", if accel { "accel" } else { "drag" }, yawo, pitch);
                c.note(cx, "MeleeAttack", s, ATTACK_FN);
                c.start_facing_bone(en, bone_h, [yawo as f32, pitch as f32], now);
            }
        }
        _ => {}
    }
    IN_PROGRESS
}

/// attack choice, UBTTask_MeleeAttack::PerformOffensiveEvaluation disasm 0x141496a7f-0x141496b78
fn choose_and_request(c: &mut BotController, cx: &mut Ctx, w: &super::body::WeaponView, dist: f64) {
    let sides = c.ally_clearance_sides(cx); // GetAllyClearanceSides rva=0x14fa080
    let mut mv = E::KICK;
    if !c.profile.p.will_brawl || K::ATTACK_BRAWL_RANGE < dist {
        // brawl (kick) only within 150
        let stab = w.stab_damage0; // StabAttack.Damage[0] (weapon +0xfa0 -> data[0])
        let strike = w.strike_damage0; // StrikeAttack.Damage[0] (weapon +0x1440)
        let p = f32r(stab / maxf(strike + stab, K::ATTACK_CHOICE_MIN_WEIGHT));
        mv = if p > cx.rng.frand(K::FRAND_SCALE) { E::STAB } else { E::RIGHT_STRIKE };
        let r = ((cx.rng.rand() as f64 * K::ATTACK_FLIP_RAND_SCALE) as i64).min(1); // 2/32767
        if r == 1 {
            let _ = E::flip_side(mv); // PerformOffensiveEvaluation 0x141496b23: result discarded (bl unchanged)
        }
        let left = E::is_left(mv);
        if (sides == 0 && left) || (sides == 1 && !left) {
            mv = E::flip_side(mv);
        }
        c.note(cx, "MeleeAttack", format!("P(stab)={:.4} sides={} -> {}", p, sides, E::move_name(mv)), ATTACK_FN);
    }
    let angle = cx.rng.rand() as f64 * K::ATTACK_ANGLE_RAND_SCALE - K::ATTACK_ANGLE_OFFSET; // 120/32767 * r - 60
    c.note(cx, "MeleeAttack", format!("RequestAttack({}, {:.3})", E::move_name(mv), angle), ATTACK_FN);
    c.request_attack(mv, angle, cx);
}

/// circling (PerformOffensiveEvaluation disasm 0x141496594-0x141496862): target bearing around the enemy = rnd * 360
/// - 180 relative to its yaw (or opposite a close ally), step at most 30 degrees per evaluation, at move_dist
fn circle(c: &mut BotController, cx: &mut Ctx, en: usize, ally: Option<usize>, dir: FVector, rnd: f64, move_dist: f64) {
    let me_loc = cx.bodies[c.body].location;
    let (en_loc, en_yaw) = (cx.bodies[en].location, cx.bodies[en].yaw);
    let mut ally_dist = K::CIRCLE_NO_ALLY_DIST; // FLT_MAX
    if let Some(a) = ally {
        if !cx.bodies[a].is_dead {
            ally_dist = (cx.bodies[a].location - me_loc).length() as f64;
        }
    }
    let cur = angle_2d(cx, dir, en_yaw);
    if cx.exe {
        return circle_exe(c, cx, en, ally, ally_dist, dir, cur as f32, rnd as f32, move_dist);
    }
    let mut target = rnd * K::CIRCLE_FULL_TURN - K::CIRCLE_HALF_TURN; // * 360 - 180
    if ally_dist < K::CIRCLE_ALLY_AVOID_DIST {
        // 300
        let a_loc = cx.bodies[ally.unwrap()].location;
        let mut a = (angle_2d(cx, safe_normal(a_loc - en_loc), en_yaw) + K::CIRCLE_HALF_TURN) % K::CIRCLE_FULL_TURN;
        if a < 0.0 {
            a += K::CIRCLE_FULL_TURN;
        }
        if a > K::CIRCLE_HALF_TURN {
            a -= K::CIRCLE_FULL_TURN;
        }
        target = a;
    }
    let step = clampf(find_delta_angle(cur, target), K::CIRCLE_STEP_MIN, K::CIRCLE_STEP_MAX); // +-30
    let dest = en_loc + rotate_yaw(dir.scale(move_dist), step);
    if !c.nav_blocked(en_loc, dest, cx) {
        c.move_to_location(dest, K::DEFAULT_ACCEPTANCE, cx.now);
    } else {
        c.move_to_location(en_loc, move_dist, cx.now);
    }
}

/// circle in the exe's single-precision instruction sequence (disasm 0x141496594-0x141496862): target = rnd * 360 -
/// 180 (mulss, subss); a close ally: Fmod(CalculateAngle2D(ally normal) + 180, 360) (rva=0x1815130), + 360 when < 0,
/// - 360 when > 180; step = FMath::ClampAngle(FindDeltaAngleDegrees(cur, target), -30, 30) (rva=0x18a27c0 /
/// 0x1480350, .rdata 0x1443247cc / 0x143fe4e34); dest = enemy + FRotator(0, step, 0).RotateVector(dir * move_dist)
#[allow(clippy::too_many_arguments)]
fn circle_exe(c: &mut BotController, cx: &mut Ctx, en: usize, ally: Option<usize>, ally_dist: f64, dir: FVector, cur: f32, rnd: f32, move_dist: f64) {
    use mordhau_core::ue::{ue_clamp_angle, ue_find_delta_angle_degrees, ue_fmod};
    let en_loc = cx.bodies[en].location;
    let en_yaw = cx.bodies[en].yaw;
    let (full, half) = (K::CIRCLE_FULL_TURN as f32, K::CIRCLE_HALF_TURN as f32);
    let mut target = rnd * full - half;
    if ally_dist < K::CIRCLE_ALLY_AVOID_DIST {
        let a_loc = cx.bodies[ally.unwrap()].location;
        let n = mordhau_core::ue::ue_safe_normal(a_loc - en_loc);
        let mut a = ue_fmod(angle_2d(cx, n, en_yaw) as f32 + half, full);
        if a < 0.0 {
            a += full;
        }
        if a > half {
            a -= full;
        }
        target = a;
    }
    let step = ue_clamp_angle(ue_find_delta_angle_degrees(cur, target), K::CIRCLE_STEP_MIN as f32, K::CIRCLE_STEP_MAX as f32);
    let off = mordhau_core::ue::ue_rotate_yaw(dir.scale(move_dist), step);
    let dest = FVector::new(off.x + en_loc.x, off.y + en_loc.y, off.z + en_loc.z);
    if !c.nav_blocked(en_loc, dest, cx) {
        c.move_to_location(dest, K::DEFAULT_ACCEPTANCE, cx.now);
    } else {
        c.move_to_location(en_loc, move_dist, cx.now);
    }
}

// ==== UBTTask_MeleeDefend ==========================================================================================
const DEFEND_FN: &str = "UBTTask_MeleeDefend::PerformDefensiveEvaluation rva=0x14940a0";

/// ExecuteTask rva=0x147fe70 / TickTask rva=0x14a5160: Perform; anything but InProgress also runs StopCrouching
fn defend_execute(c: &mut BotController, cx: &mut Ctx) -> i32 {
    let r = defend_perform(c, cx);
    if r != IN_PROGRESS {
        stop_crouching_task(c, cx);
    }
    r
}

/// StopCrouching rva=0x14a3460: controller's pawn is an alive AMordhauCharacter -> bWantsCrouch (+0xde4) = 0
fn stop_crouching_task(c: &BotController, cx: &mut Ctx) {
    if !cx.bodies[c.body].is_dead {
        cx.bodies[c.body].wants_crouch = false;
    }
}

fn defend_perform(c: &mut BotController, cx: &mut Ctx) -> i32 {
    let me_i = c.body;
    if !cx.bodies[me_i].has_sys {
        return FAILED; // AIOwner / Pawn casts / MotionSystemComponent (+0x688) null
    }
    let own = cx.bodies[me_i].pawn.motion.clone();
    let own_parry = own.is_parry; // Cast<UParryMotion>(Motion) (StaticClass 0x141710970)
    let now = cx.now; // GetWorld()->TimeSeconds (+0x598)
    c.get_motion_based_random(cx);
    c.start_sprinting(cx); // AMordhauCharacter::StartSprinting rva=0x156dcc0
    let my_loc = cx.bodies[me_i].location;
    let my_fwd = body_forward(cx, me_i);
    let mut pending = false; // bVar9: an attack is coming but not yet in threat range
    for en in c.perceived_enemies(cx) {
        // AMordhauAIController::GetPerceivedEnemies rva=0x14fd110
        let enb = &cx.bodies[en];
        if !enb.has_sys {
            continue;
        }
        let m = enb.pawn.motion.clone();
        if !m.is_attack || m.stage == E::RECOVERY {
            continue; // +0x10e9 Stage != 2
        }
        let en_name = enb.name.clone();
        let windup = m.stage == E::WINDUP; // bVar8
        if m.stage == E::RELEASE && m.has_hit && enb.ignore_cache.contains(&me_i) {
            c.note(cx, "MeleeDefend", format!("{}: Release, already hit me (Weapon.ActorIgnoreCache) -> ignore", en_name), DEFEND_FN);
            continue;
        }
        // threat / reach ranges (LAB_1414943af)
        let me = &cx.bodies[me_i];
        let q = cx.q;
        let time_left = q(m.release_end - now); // ReleaseEnd (+0x1094) - now
        let threat = q(q(q(q(me.scaled_radius()) + q(enb.scaled_radius())) + q(q(enb.weapon_length * K::WEAPON_LENGTH_TO_CM) * enb.mesh_scale_x))
            + q(time_left * enb.threat_speed()));
        let reach = q(q(me.threat_speed() * time_left) + threat);
        let en_loc = enb.location;
        let d = safe_normal(my_loc - en_loc);
        let neg = FVector::new(-d.x, -d.y, -d.z);
        let not_in_front = (neg.dot(my_fwd) as f64) < K::DEFEND_IN_FRONT_DOT; // bVar7, 0.3
        let dx = en_loc.x - my_loc.x;
        let dy = en_loc.y - my_loc.y;
        let dist2d = (dx * dx + dy * dy).sqrt() as f64;
        if (d.dot(body_forward(cx, en)) as f64) < K::DEFEND_ENEMY_FACING_DOT || dist2d > reach {
            // -0.5: enemy faces away
            let s = format!("{} {}: not facing me or dist {:.1} > reach {:.1} -> ignore", en_name, E::stage_name(m.stage), dist2d, reach);
            c.note(cx, "MeleeDefend", s, DEFEND_FN);
            continue;
        }
        if dist2d > threat {
            pending = true;
            let s = format!("{}: dist {:.1} in reach {:.1}, outside threat {:.1} -> wait", en_name, dist2d, reach, threat);
            c.note(cx, "MeleeDefend", s, DEFEND_FN);
            continue;
        }
        if !(not_in_front || !windup || !c.profile.p.will_gamble) {
            c.note(cx, "MeleeDefend", format!("{}: Windup in front and bWillGamble -> ignore (gamble)", en_name), DEFEND_FN);
            continue;
        }
        // footwork: only against the first enemy attack motion it was chosen for (LastFootworkingEnemyMotion +0xe8)
        let mut footwork = c.profile.p.will_footwork; // +0xe4
        if footwork {
            match c.profile.last_footworking_enemy_motion {
                None => c.profile.last_footworking_enemy_motion = Some(m.id),
                Some(id) if id != m.id => footwork = false,
                _ => {}
            }
        }
        if not_in_front && !footwork {
            c.note(cx, "MeleeDefend", format!("{}: attack not in front of me -> ignore", en_name), DEFEND_FN);
            continue;
        }
        let perfect = c.profile.p.will_perfect_parry; // +0xdc
        let delay = if perfect { 0.0 } else { c.profile.p.parry_timing_random }; // +0xe0
        let zdiff = (en_loc.z - my_loc.z).abs() as f64;
        let may_move = c.get_move_status() == PathStatus::Idle || zdiff < K::DEFEND_MOVE_MAX_ZDIFF; // GetMoveStatus, 100
        let s = format!(
            "{} {} {}: threat (dist {:.1} <= {:.1}), footwork={} perfect={} delay={:.4}",
            en_name,
            E::move_name(m.mv),
            E::stage_name(m.stage),
            dist2d,
            threat,
            footwork,
            perfect,
            delay
        );
        c.note(cx, "MeleeDefend", s, DEFEND_FN);
        if !footwork || !c.profile.p.will_footwork_with_crouch || now <= q(m.windup_end + K::DEFEND_CROUCH_DELAY) {
            c.stop_crouching(cx); // AMordhauCharacter::StopCrouching rva=0x156ddc0
        } else {
            c.start_facing_actor(en, K::DEFEND_FOOTWORK_FACING, [0.0, 0.0], now); // -70
            c.start_crouching(cx); // bWantsCrouch_SetBit / StartCrouching rva=0x156dc90
        }
        if may_move {
            let target = cx.bodies[en].trace_start; // Weapon (+0x10c8) CurrentTraceStart (+0xd48)
            let back = maxf(q(q(reach + K::DEFEND_BACK_OFF_MARGIN) * c.profile.p.back_off_factor_during_defense), K::DEFEND_BACK_OFF_MIN);
            let mut k = back;
            if footwork {
                k = K::DEFEND_BACK_OFF_MIN;
                if !c.profile.p.will_footwork_with_crouch {
                    // +0xe5
                    c.start_facing_movement(K::DEFEND_FOOTWORK_FACING);
                    k = back;
                }
            }
            let dest = target + safe_normal(my_loc - target).scale(k);
            if !c.nav_blocked(en_loc, dest, cx) {
                // UNavigationSystemV1::NavigationRaycast
                c.move_to_location(dest, K::DEFAULT_ACCEPTANCE, now);
            } else {
                c.move_to_location(en_loc, k, now);
            }
        }
        if footwork {
            c.note(cx, "MeleeDefend", "footwork instead of parry -> InProgress".into(), DEFEND_FN);
            return IN_PROGRESS;
        }
        if c.profile.p.will_fall_for_feint && m.attack_type == E::ATTACK_MORPH {
            // +0xd4, Type (+0x108d) == 4
            c.note(cx, "MeleeDefend", "morph and bWillFallForFeint -> Succeeded (FallForFeint parries it)".into(), DEFEND_FN);
            return SUCCEEDED;
        }
        c.start_facing_location(en_loc + FVector::new(0.0, 0.0, K::DEFEND_FACING_HEIGHT as f32));
        if own_parry {
            c.note(cx, "MeleeDefend", "already parrying -> InProgress".into(), DEFEND_FN);
            return IN_PROGRESS;
        }
        // parry once now > WindupEnd + GetEarlyReleaseDuration + delay - 0.1 (_DAT_143fe4dfc)
        let at = q(q(q(m.early_release_duration + m.windup_end) + delay) - K::DEFEND_PARRY_LEAD);
        if now <= at {
            c.note(cx, "MeleeDefend", format!("now {:.4} <= parry time {:.4} -> InProgress", now, at), DEFEND_FN);
            return IN_PROGRESS;
        }
        if perfect && !blade_reaches(&cx.bodies[me_i], &cx.bodies[en]) {
            c.note(cx, "MeleeDefend", "perfect parry: blade sweep misses my capsule -> InProgress".into(), DEFEND_FN);
            return IN_PROGRESS;
        }
        let bt = if E::is_left(m.mv) { E::BLOCK_ALT_REGULAR } else { E::BLOCK_REGULAR };
        c.note(cx, "MeleeDefend", format!("RequestParry(IsLeft({}) = {})", E::move_name(m.mv), bt), DEFEND_FN);
        c.request_parry(bt, cx); // AMordhauCharacter::RequestParry rva=0x1564860 (EBlockType, true)
        if cx.bodies[me_i].pawn.motion.is_parry {
            return IN_PROGRESS;
        }
        return FAILED;
    }
    if pending {
        return IN_PROGRESS;
    }
    if own.kind.starts_with("Flinch") || own.kind.starts_with("Blocked") {
        return FAILED; // UFlinchMotion / UBlockedMotion StaticClass checks
    }
    SUCCEEDED
}

/// perfect parry (LAB_141494d80): step t = 0, 20, ... < 160 (_DAT_143fe4e30, _DAT_144324788) along
/// dir = normal(CurrentTraceEnd - PreviousTraceEnd); line CurrentTraceStart -> CurrentTraceEnd + dir * t against my
/// capsule (UPrimitiveComponent::LineTraceComponent, capsule vtable +0x848; engine test UNCONFIRMED)
fn blade_reaches(me: &BotBody, en: &BotBody) -> bool {
    let dir = safe_normal(en.trace_end - en.prev_trace_end);
    let mut t = 0.0;
    while t < K::DEFEND_SWEEP_MAX {
        if segment_hits_capsule(en.trace_start, en.trace_end + dir.scale(t), me.location, me.capsule_half_height * me.capsule_scale_min, me.scaled_radius()) {
            return true;
        }
        t += K::DEFEND_SWEEP_STEP;
    }
    false
}

// ==== UBTTask_FallForFeint =========================================================================================
// A bot that rolled bWillFallForFeint parries a feint or a morph of the enemy it is facing: a morph at once, a feint
// 0.2 s after the feint began; while a real attack is still more than 0.1 s from WindupEnd it keeps watching.
const FEINT_FN: &str = "UBTTask_FallForFeint::PerformFeintEvaluation rva=0x1495720";

fn feint_perform(c: &mut BotController, cx: &mut Ctx) -> i32 {
    let me_i = c.body;
    if !cx.bodies[me_i].has_sys {
        return FAILED;
    }
    c.get_motion_based_random(cx);
    if !c.profile.p.will_fall_for_feint {
        return SUCCEEDED; // +0xd4
    }
    let now = cx.now;
    let my_loc = cx.bodies[me_i].location;
    let mut watching = false; // cVar7
    for en in c.perceived_enemies(cx) {
        if c.currently_facing_actor() != Some(en) {
            continue; // GetCurrentlyFacingActor rva=0x14fafd0 (+0x33c)
        }
        let enb = &cx.bodies[en];
        let d = safe_normal(my_loc - enb.location);
        if (d.dot(body_forward(cx, en)) as f64) < K::FEINT_ENEMY_FACING_DOT {
            continue; // -0.5: enemy not facing me
        }
        if !enb.has_sys {
            continue;
        }
        let m = enb.pawn.motion.clone();
        let mut parry_now = false; // bVar5
        let mut wait = false; // bVar6
        if m.is_feinted {
            // UFeintedMotion StaticClass 0x14168e7c0
            let t = (cx.q)(m.start_time + K::FEINT_REACT_DELAY); // StartTime (+0x4c) + 0.2
            parry_now = t < now;
            wait = now <= t;
        } else if m.is_attack {
            if m.attack_type == E::ATTACK_MORPH {
                parry_now = true; // Type (+0x108d) == 4
            } else {
                if (cx.q)(m.windup_end - K::FEINT_WATCH_LEAD) <= now {
                    continue; // WindupEnd - 0.1
                }
                wait = true;
            }
        } else {
            continue;
        }
        if ((enb.location - my_loc).length_squared() as f64) <= K::FEINT_RANGE_SQ {
            // 32400 = 180^2
            if parry_now {
                let s = format!("{} {} -> RequestParry(Regular), Succeeded", enb.name, m.kind);
                c.note(cx, "FallForFeint", s, FEINT_FN);
                c.request_parry(E::BLOCK_REGULAR, cx); // RequestParry(0, true)
                return SUCCEEDED;
            }
            if wait {
                watching = true;
            }
        }
    }
    if watching {
        c.note(cx, "FallForFeint", "watching a windup / fresh feint -> InProgress".into(), FEINT_FN);
        return IN_PROGRESS;
    }
    SUCCEEDED
}

// ==== UBTTask_SwitchEquipment ======================================================================================
// Walks the inventory (AMordhauCharacter +0x11e8): the first item that qualifies is either already in a hand
// (Succeeded) or gets an equip motion (InProgress); none qualifies -> Failed. While an equip motion runs ->
// InProgress. Melee (bMelee): bCanAttack (+0xcb9) and an AMordhauWeapon; the ranged branch (bAllowFire +0xcbc, vtable
// +0x6f0, ammo +0x648) is not ported (UNCONFIRMED: `ranged` flag). Allowed / NotAllowed lists matched by class name.
// The equip motion (FNetMotion MotionType 8, slot index) is requested as an event.
const SWITCH_FN: &str = "UBTTask_SwitchEquipment::PerformSwitchingTask rva=0x1497830";

fn item_matches(item: &super::body::InventoryItem, names: &[String]) -> bool {
    let mut ids: Vec<String> = Vec::new();
    match &item.weapon {
        Some(w) => {
            ids.push(w.id.clone());
            ids.extend(w.chain.iter().cloned());
            ids.push(w.native_class.clone());
        }
        None => ids.push(String::new()),
    }
    if item.is_fists {
        ids.push("FistsWeapon".into());
    }
    let file = |s: &str| s.rsplit('/').next().unwrap_or("").to_string();
    names.iter().any(|n| ids.iter().any(|i| &file(i) == n))
}

fn switch_perform(c: &mut BotController, cx: &mut Ctx, p: &TaskParams) -> i32 {
    let me_i = c.body;
    if !cx.bodies[me_i].has_sys {
        return FAILED;
    }
    if cx.bodies[me_i].pawn.motion.kind.starts_with("Equip") {
        return IN_PROGRESS; // UEquipMotion StaticClass 0x14168f900
    }
    let inv = cx.bodies[me_i].inventory.clone();
    for (i, item) in inv.iter().enumerate() {
        let Some(w) = &item.weapon else { continue };
        let ok = if p.b_melee { w.b_can_attack } else { item.ranged };
        if !ok {
            continue;
        }
        if !p.allowed_subclasses.is_empty() && !item_matches(item, &p.allowed_subclasses) {
            continue;
        }
        if !p.not_allowed_subclasses.is_empty() && item_matches(item, &p.not_allowed_subclasses) {
            continue;
        }
        // the first matching item decides (rva=0x1497830 @0x141497b8e): already the RightHandEquipment (+0x1200) or
        // the LeftHandEquipment (+0x11f8) -> Succeeded; else CanInitiateMotion(UEquipmentSwitchMotion) and
        // AssignNetMotion {8, slot} -> InProgress (InProgress also when the motion can't start: the host's equip
        // event is then ignored by the motion system)
        if i == cx.bodies[me_i].right_hand || cx.bodies[me_i].left_hand == Some(i) {
            c.note(cx, "SwitchEquipment", format!("{} already in hand -> Succeeded", w.id), SWITCH_FN);
            return SUCCEEDED;
        }
        c.note(cx, "SwitchEquipment", format!("equip slot {} ({}) -> InProgress", i, w.id), SWITCH_FN);
        c.events.push(BotEvent { t: cx.now, kind: "equip", id: i as i64, forced: false }); // AssignNetMotion {8, slot}
        return IN_PROGRESS;
    }
    FAILED
}

// ==== UBTTask_VoiceOrEmote =========================================================================================
// Instant task: one rand() against Chance first (fails the task when above it); then, cooldowns allowing, a random
// entry of VoiceCommandsList / EmotesList (presentation: events for the host). The cooldown timestamps live on the game
// mode (+0x770 voice, +0x774 emote): the shared WorldState here.
const VOICE_FN: &str = "UBTTask_VoiceOrEmote::ExecuteTask rva=0x147fed0";

/// the list index: int(f32(rand() & 0x7fff) * 3.051851e-05 * f32(n)), min n - 1 (rva=0x147fed0; the exe multiplies in
/// binary32: Precision::Exe; the reference in f64)
fn voice_pick(cx: &mut Ctx, n: i64) -> i64 {
    let r = cx.rng.rand();
    let i = if cx.exe { ((r as f32) * (K::VOICE_PICK_RAND_SCALE as f32) * (n as f32)) as i64 } else { (r as f64 * K::VOICE_PICK_RAND_SCALE * n as f64) as i64 };
    i.min(n - 1)
}

fn voice_execute(c: &mut BotController, cx: &mut Ctx, p: &TaskParams) -> i32 {
    let r = cx.rng.frand(K::FRAND_SCALE);
    if !(r <= p.chance) {
        return FAILED;
    }
    if cx.bodies[c.body].is_dead {
        return FAILED;
    }
    let now = cx.now;
    let gcd = p.global_cooldown;
    if !p.voice_commands.is_empty() {
        let n = p.voice_commands.len() as i64;
        let i = voice_pick(cx, n);
        // game mode +0x770 last voice time; voice component +0xe8 last voice time + 3.0 (_DAT_143fe4e10)
        if gcd + cx.ws.last_voice < now && c.last_voice_time + K::VOICE_MIN_INTERVAL < now {
            cx.ws.last_voice = now;
            c.last_voice_time = now;
            let id = p.voice_commands[i as usize];
            c.events.push(BotEvent { t: now, kind: "voice", id, forced: false }); // RequestVoiceCommand(id, 0)
            c.note(cx, "VoiceOrEmote", format!("voice {}", id), VOICE_FN);
        }
    }
    if !p.emotes.is_empty() {
        let n = p.emotes.len() as i64;
        let j = voice_pick(cx, n);
        if gcd + cx.ws.last_emote < now {
            cx.ws.last_emote = now;
            let id = p.emotes[j as usize];
            // bForceEmote: AssignNetMotion {MotionType 0x0a, id}; else UMotionSystemComponent::RequestEmote
            c.events.push(BotEvent { t: now, kind: "emote", id, forced: p.b_force_emote });
            c.note(cx, "VoiceOrEmote", format!("emote {}", id), VOICE_FN);
        }
    }
    SUCCEEDED
}

// ==== BTTask_BackOff_C =============================================================================================
// The combat tree's spacing / circling move, from its Blueprint bytecode (statement indices in brackets):
//   ReceiveExecuteAI [1997]: only when RequiresInitialization: clear it; with a closest enemy: OriginalAngle = yaw of
//     MakeRotFromZX(Up, Normal(pawn - enemy, 1e-4)), RightOffset = 0, RandomSide = RandomFloat > 0.5 ? 1 : -1
//   ReceiveTickAI [27]: closest enemy, and (GetMotionBasedRandom < 0.1 or bIsClosestEnemySaturated), else [1774]
//     RequiresInitialization = true, FinishExecute(true). Weaker than the enemy (our Health < byte(round(enemy Health *
//     0.5)) or our Stamina < byte(round(enemy Stamina * 0.5))): Distance = RandomFloat * 300 + 700, StopSprinting;
//     else only when saturated: Distance = RandomFloat * 500 + 200, StartSprinting (not saturated -> [1774]).
//     Then [1115]: RightOffset += dt * RandomSide; MoveToLocation(enemy + rotate((1,0,0), yaw OriginalAngle +
//     RightOffset * 360 * 0.33) * 700, acceptance -1); StartFacingActor(enemy, 25, (0,0)); FinishExecute(false).
//     Distance is computed and never read (the move radius is the literal 700).
impl BackOff {
    fn execute(&mut self, c: &mut BotController, cx: &mut Ctx) -> i32 {
        if self.requires_initialization {
            self.requires_initialization = false;
            if let Some(en) = c.closest_enemy(cx) {
                let away = cx.bodies[c.body].location - cx.bodies[en].location;
                // UKismetMathLibrary::Normal(v, 1e-4) = FVector::GetSafeNormal: zero when |v|^2 < tolerance
                if cx.exe {
                    // Normal(v, 1e-4) = GetSafeNormal(Tolerance); MakeRotFromZX rva=0x18b3d80 + FMatrix::Rotator rva=0x18bdb00
                    let n = mordhau_core::ue::ue_safe_normal_tol(away, cx.k.f("ai_backoff_normal_tol") as f32);
                    self.original_angle = mordhau_core::ue::ue_make_rot_from_zx_yaw(FVector::new(0.0, 0.0, 1.0), n) as f64;
                } else {
                    let n = if (away.length_squared() as f64) >= cx.k.f("ai_backoff_normal_tol") { away.normalized() } else { FVector::ZERO };
                    self.original_angle = (n.y as f64).atan2(n.x as f64).to_degrees(); // MakeRotFromZX(Up, n): yaw of n
                }
                self.right_offset = 0.0;
                self.random_side = if c.random_float > cx.k.f("ai_backoff_side_split") { 1.0 } else { -1.0 };
            }
        }
        IN_PROGRESS
    }

    fn tick(&mut self, c: &mut BotController, cx: &mut Ctx, dt: f64) -> i32 {
        let en = c.closest_enemy(cx);
        let me_i = c.body;
        let en = match en {
            Some(e) if cx.bodies[me_i].has_sys => e,
            _ => return self.done(c, cx, true),
        };
        let mr = c.get_motion_based_random(cx);
        if !(mr < cx.k.f("ai_backoff_motion_random") || c.closest_enemy_saturated) {
            return self.done(c, cx, true);
        }
        let frac = cx.k.f("ai_backoff_enemy_frac");
        let half = |v: i64| (((v as f64) * frac + 0.5).floor() as i64) & 0xff;
        let (me, enb) = (&cx.bodies[me_i], &cx.bodies[en]);
        let weaker = me.health_byte() < half(enb.health_byte()) || me.stamina_byte() < half(enb.stamina_byte());
        if weaker {
            self.distance = c.random_float * cx.k.f("ai_backoff_weak_rand") + cx.k.f("ai_backoff_weak_base");
            cx.bodies[me_i].wants_sprint = false; // StopSprinting
        } else if c.closest_enemy_saturated {
            self.distance = c.random_float * cx.k.f("ai_backoff_sat_rand") + cx.k.f("ai_backoff_sat_base");
            c.start_sprinting(cx);
        } else {
            return self.done(c, cx, true);
        }
        let (yaw, dest) = if cx.exe {
            // Kismet floats [1115]-[1553]: RightOffset = dt * RandomSide + RightOffset; yaw = OriginalAngle + (RightOffset *
            // 360) * 0.33; (1, 0, 0) >> MakeRotator(0, 0, yaw) (FRotator::RotateVector rva=0x18bd9b0) * 700 + location
            let ro = (dt as f32 * self.random_side as f32) + self.right_offset as f32;
            self.right_offset = ro as f64;
            let y = self.original_angle as f32 + (ro * cx.k.f("ai_backoff_turn_deg") as f32) * cx.k.f("ai_backoff_turn_scale") as f32;
            let u = mordhau_core::ue::ue_rotate_yaw(FVector::new(1.0, 0.0, 0.0), y).scale(cx.k.f("ai_backoff_radius"));
            let l = cx.bodies[en].location;
            ((y as f64).to_radians(), FVector::new(u.x + l.x, u.y + l.y, u.z + l.z))
        } else {
            self.right_offset += dt * self.random_side;
            let yaw = (self.original_angle + self.right_offset * cx.k.f("ai_backoff_turn_deg") * cx.k.f("ai_backoff_turn_scale")).to_radians();
            let unit = FVector::new(yaw.cos() as f32, yaw.sin() as f32, 0.0);
            (yaw, cx.bodies[en].location + unit.scale(cx.k.f("ai_backoff_radius")))
        };
        c.move_to_location(dest, cx.k.f("ai_backoff_acceptance"), cx.now);
        let now = cx.now;
        c.start_facing_actor(en, cx.k.f("ai_backoff_face_up"), [0.0, 0.0], now);
        let s = format!("circle to yaw {:.1} (distance {:.0}, {})", yaw.to_degrees(), self.distance, if weaker { "weaker" } else { "saturated" });
        c.note(cx, "BackOff", s, "BTTask_BackOff_C ReceiveTickAI");
        FAILED // FinishExecute(false)
    }

    fn done(&mut self, c: &mut BotController, cx: &Ctx, ok: bool) -> i32 {
        self.requires_initialization = true;
        let s = format!("no back-off -> {}", if ok { "Succeeded" } else { "Failed" });
        c.note(cx, "BackOff", s, "BTTask_BackOff_C ReceiveTickAI");
        if ok {
            SUCCEEDED
        } else {
            FAILED
        }
    }
}

// ==== BTTask_FindRandomLocation_C / BTTask_FindUnstuckSpot_C =======================================================
/// ReceiveExecuteAI [1059]: EnemyPositions = the location of every BP_MordhauCharacter that is not dead, not our pawn
/// and not (team mode and the same Team byte) [15..377]; GetRandomReachablePointInRadius(GetCentroid(EnemyPositions),
/// 1000) - queried twice, the second result used [686, 754] -> blackboard TargetLocation, FinishExecute(true); no point
/// -> ClearBlackboardValue(TargetLocation), FinishExecute(false). GetCentroid of an empty array: zero (UNCONFIRMED).
fn find_random_location(c: &mut BotController, cx: &mut Ctx, p: &TaskParams) -> i32 {
    let my_team = cx.bodies[c.body].team & 0xff;
    let mut cen = FVector::ZERO;
    let mut n = 0usize;
    for (i, b) in cx.bodies.iter().enumerate() {
        if b.gone || b.is_dead || i == c.body || (c.team_mode && (b.team & 0xff) == my_team) {
            continue;
        }
        cen = cen + b.location;
        n += 1;
    }
    if n > 0 {
        let s = n as f32;
        cen = FVector::new(cen.x / s, cen.y / s, cen.z / s);
    }
    let r = cx.k.f("ai_find_random_radius");
    if c.random_reachable_point(cen, r, cx).is_some() {
        if let Some(pt) = c.random_reachable_point(cen, r, cx) {
            c.blackboard.insert(p.target_location_key.clone(), BbVal::Vec(pt));
        }
        c.note(cx, "FindRandomLocation", "target near the enemies' centroid".into(), "BTTask_FindRandomLocation_C");
        return SUCCEEDED;
    }
    c.blackboard.remove(&p.target_location_key);
    FAILED
}

/// ReceiveExecuteAI [26]: GetRandomReachablePointInRadius(pawn location, 500) - twice, the second result used ->
/// TargetLocation, FinishExecute(true); none -> clear TargetLocation, FinishExecute(false)
fn find_unstuck_spot(c: &mut BotController, cx: &mut Ctx, p: &TaskParams) -> i32 {
    let r = cx.k.f("ai_unstuck_radius");
    let here = cx.bodies[c.body].location;
    if c.random_reachable_point(here, r, cx).is_some() {
        if let Some(pt) = c.random_reachable_point(here, r, cx) {
            c.blackboard.insert(p.target_location_key.clone(), BbVal::Vec(pt));
        }
        return SUCCEEDED;
    }
    c.blackboard.remove(&p.target_location_key);
    FAILED
}

// ==== BTTask_FindHordeTask_C (Blueprint bytecode, Horde/AI/BTTask_FindHordeTask; statement indices in brackets) ======
// ReceiveExecuteAI -> ubergraph [1568]: the controller a BP_MordhauAIController and the pawn a BP_HordeEnemy, else
// [672] ClearBlackboardValue(TargetLocation), FinishExecute(false). CurrentTask valid [1726] -> TargetLocation =
// CurrentTask.GetLocationTarget(pawn) [1791-1867], FinishExecute(true) [15]. Else EnemyPositions cleared [1901]; the
// GameState a MordhauGameState (else [672]); GetAllActorsOfClass(BP_MordhauCharacter_C) [2053]; every actor that is not
// (bIsTeamMode && same Team) || itself || GetIsDead adds its location [27-1567]. None -> [672]. Else
// GetRandomReachablePointInRadius(EnemyPositions[RandomIntegerInRange(0, n - 1)], 500, team filter) [704-951]: none
// -> [672]; else the same again [1019-1221] and its point (not checked) -> TargetLocation, FinishExecute(true).
// GetAllActorsOfClass order = the body order here (UNCONFIRMED: the engine's actor iterator order).
const FIND_HORDE_FN: &str = "BTTask_FindHordeTask_C ReceiveExecuteAI";

fn find_horde_task(c: &mut BotController, cx: &mut Ctx, p: &TaskParams) -> i32 {
    let me = c.body;
    if let Some(t) = cx.bodies[me].horde_task_target {
        c.blackboard.insert(p.target_location_key.clone(), BbVal::Vec(t));
        c.note(cx, "FindHordeTask", "CurrentTask location -> Succeeded".into(), FIND_HORDE_FN);
        return SUCCEEDED;
    }
    let my_team = cx.bodies[me].team & 0xff;
    let pos: Vec<FVector> = cx
        .bodies
        .iter()
        .enumerate()
        .filter(|(i, b)| !b.gone && !((c.team_mode && (b.team & 0xff) == my_team) || *i == me || b.is_dead))
        .map(|(_, b)| b.location)
        .collect();
    let fail = |c: &mut BotController, cx: &mut Ctx| {
        c.blackboard.remove(&p.target_location_key);
        c.note(cx, "FindHordeTask", "no enemy position -> Failed".into(), FIND_HORDE_FN);
        FAILED
    };
    if pos.is_empty() {
        return fail(c, cx);
    }
    let n = pos.len() as i64;
    let i = crate::kismet_rand::random_integer_in_range(cx.rng, 0, n - 1);
    if c.random_reachable_point(pos[i as usize], 500.0, cx).is_none() {
        return fail(c, cx);
    }
    let j = crate::kismet_rand::random_integer_in_range(cx.rng, 0, n - 1);
    match c.random_reachable_point(pos[j as usize], 500.0, cx) {
        Some(pt) => {
            c.blackboard.insert(p.target_location_key.clone(), BbVal::Vec(pt));
        }
        // the second result is not checked [1275]: an unset out-param is the zero vector
        None => {
            c.blackboard.insert(p.target_location_key.clone(), BbVal::Vec(FVector::ZERO));
        }
    }
    c.note(cx, "FindHordeTask", format!("a point near enemy {j} of {n} -> Succeeded"), FIND_HORDE_FN);
    SUCCEEDED
}

/// BTService_HordePerceptionUpdate_C ReceiveTickAI -> ubergraph [916] (Horde/AI/BTService_HordePerceptionUpdate):
/// pawn a BP_HordeEnemy and valid (else nothing); KillObj = the horde mode's KillObjectiveCached [1138]; controller a
/// BP_MordhauAIController (else nothing); SetClosestEnemyOverride(GetEnragedTarget) [1258-1303] (BP_HordeEnemy
/// GetEnragedTarget: RageTarget valid and alive, else null); TargetEnemy = GetClosestEnemy [1344].
///   [10] TargetEnemy valid: FindPathToLocationSynchronously(pawn, TargetEnemy) [153]; valid and not partial -> [630];
///        else CurrentTask valid -> [432] ClearBlackboardValue(bPerceivesEnemy); else KillObj valid -> [570]
///        SetClosestEnemyOverride(KillObj), TargetEnemy = KillObj -> [630]; else -> [630].
///   [457] no TargetEnemy: CurrentTask valid -> [432]; KillObj valid -> [570]; else [432].
///   [630] bPerceivesEnemy = true, ClosestEnemyDistance = VSize(TargetEnemy - pawn) [651-834].
/// No navigation answer (BotHost::path_complete None): the path counts as complete (UNCONFIRMED stand-in).
pub fn horde_perception_update(c: &mut BotController, cx: &mut Ctx, perceives_key: &str, distance_key: &str) {
    let me = c.body;
    if cx.bodies[me].gone || cx.bodies[me].is_dead {
        return;
    }
    let kill_obj = cx.ws.kill_objective.filter(|&k| !cx.bodies[k].gone);
    let enraged = cx.bodies[me].rage_target.filter(|&t| !cx.bodies[t].gone && !cx.bodies[t].is_dead);
    c.closest_enemy_override = enraged;
    let mut target = c.closest_enemy(cx);
    let has_task = cx.bodies[me].horde_task_target.is_some();
    let clear = |c: &mut BotController| {
        c.blackboard.insert(perceives_key.to_string(), BbVal::Bool(false));
    };
    match target {
        Some(t) => {
            let (a, b) = (cx.bodies[me].location, cx.bodies[t].location);
            let ok = cx.host.path_complete(a, b).unwrap_or(true);
            if !ok {
                if has_task {
                    return clear(c);
                }
                if let Some(k) = kill_obj {
                    c.closest_enemy_override = Some(k);
                    target = Some(k);
                }
            }
        }
        None => {
            if has_task {
                return clear(c);
            }
            match kill_obj {
                Some(k) => {
                    c.closest_enemy_override = Some(k);
                    target = Some(k);
                }
                None => return clear(c),
            }
        }
    }
    let t = target.unwrap();
    c.blackboard.insert(perceives_key.to_string(), BbVal::Bool(true));
    let d = (cx.bodies[t].location - cx.bodies[me].location).length() as f64;
    c.blackboard.insert(distance_key.to_string(), BbVal::Float(d));
}

// ==== BTTask_MoveToDestination_C ===================================================================================
// ReceiveExecuteAI [2985]: Init (bGoToActor = TargetActor key set), ResetTimeAndDistance.
// ReceiveTickAI [15]: pawn invalid -> FinishExecute(true); a ClimbingMotion -> keep waiting.
//   HasPath [384]: target moved more than 300 cm from MoveTargetLocation -> SetPath [567]; KeepMoving else ->
//     StopMovement, Succeeded [2539]. CurrentMovementDistance += |LastActorLocation - pawn|; > 100 ->
//     ResetTimeAndDistance [916]. CurrentMovementTime += dt; + dt >= 2 (stuck) [1098]: a random reachable point within
//     1000 -> MoveToLocation(it), keep running; none -> FinishExecute(false) [1507]. Door / destroyable handling
//     [1565..2538] (none in the port).
//   no path [2601]: IsMovePending -> return; target farther than AcceptableRadius -> SetPath; else FinishExecute(true).
//   HasPath = the controller's path following is Moving (UNCONFIRMED mapping).
// SetPath: StartFacingMovement(0), ResetTimeAndDistance, LastActorLocation = pawn; Stamina == 100 and (Health == 100
//   or !bAllowHealthRegen) and !ForceWalk -> StartSprinting else StopSprinting; MoveTargetLocation = GetTargetLoc;
//   UseMidpoint ? MoveToLocationWithRandomMidpoint : MoveToLocation (acceptance -1); result 0 -> Failed, 1 -> Succeeded.
// ResetTimeAndDistance: time 0, distance 0; facing mode != 0 -> StartFacingMovement(0).
impl MoveTo {
    fn execute(&mut self, c: &mut BotController, cx: &mut Ctx, p: &TaskParams) -> i32 {
        self.go_to_actor = matches!(c.blackboard.get(&p.target_actor_key), Some(BbVal::Body(_)));
        self.reset(c);
        let _ = cx;
        IN_PROGRESS
    }

    fn target(&self, c: &BotController, cx: &Ctx, p: &TaskParams) -> FVector {
        if self.go_to_actor {
            if let Some(BbVal::Body(a)) = c.blackboard.get(&p.target_actor_key) {
                return cx.bodies[*a].location;
            }
        }
        match c.blackboard.get(&p.target_location_key) {
            Some(BbVal::Vec(v)) => *v,
            _ => FVector::ZERO,
        }
    }

    fn reset(&mut self, c: &mut BotController) {
        self.current_movement_time = 0.0;
        self.current_movement_distance = 0.0;
        if c.facing_mode != Facing::Movement {
            c.start_facing_movement(0.0);
        }
    }

    fn set_path(&mut self, c: &mut BotController, cx: &mut Ctx, p: &TaskParams) -> i32 {
        c.start_facing_movement(0.0);
        self.reset(c);
        self.last_actor_location = cx.bodies[c.body].location;
        let full = cx.k.i("ai_moveto_sprint_full");
        let me = &cx.bodies[c.body];
        if me.stamina_byte() == full && (me.health_byte() == full || !self.allow_health_regen) && !p.force_walk {
            c.start_sprinting(cx);
        } else {
            cx.bodies[c.body].wants_sprint = false;
        }
        self.move_target_location = self.target(c, cx, p);
        let dest = self.move_target_location;
        let r = if p.use_midpoint {
            c.move_to_location_with_random_midpoint(dest, -1.0, cx)
        } else {
            c.move_to_location(dest, -1.0, cx.now);
            2
        };
        match r {
            0 => FAILED,
            1 => SUCCEEDED,
            _ => IN_PROGRESS,
        }
    }

    fn keep_moving(&self, c: &BotController, cx: &Ctx, p: &TaskParams) -> bool {
        ((cx.bodies[c.body].location - self.target(c, cx, p)).length_squared() as f64) > p.acceptable_radius * p.acceptable_radius
    }

    fn tick(&mut self, c: &mut BotController, cx: &mut Ctx, p: &TaskParams, dt: f64) -> i32 {
        if cx.bodies[c.body].is_dead {
            return SUCCEEDED;
        }
        if cx.bodies[c.body].has_sys && cx.bodies[c.body].pawn.motion.kind == "Climbing" {
            return IN_PROGRESS;
        }
        if c.get_move_status() == PathStatus::Moving {
            // HasPath
            let rp = cx.k.f("ai_moveto_repath_dist");
            let t = self.target(c, cx, p);
            if ((t - self.move_target_location).length_squared() as f64) > rp * rp {
                let r = self.set_path(c, cx, p);
                if r != IN_PROGRESS {
                    return r;
                }
            }
            if !self.keep_moving(c, cx, p) {
                c.stop_movement();
                return SUCCEEDED;
            }
            let here = cx.bodies[c.body].location;
            self.current_movement_distance += (self.last_actor_location - here).length() as f64;
            self.last_actor_location = here;
            if self.current_movement_distance > cx.k.f("ai_moveto_progress_dist") {
                self.reset(c);
            }
            self.current_movement_time += dt;
            if self.current_movement_time + dt >= cx.k.f("ai_moveto_stuck_s") {
                let ur = cx.k.f("ai_moveto_unstuck_radius");
                if c.random_reachable_point(here, ur, cx).is_some() {
                    if let Some(pt) = c.random_reachable_point(here, ur, cx) {
                        c.move_to_location(pt, -1.0, cx.now);
                    }
                    return IN_PROGRESS;
                }
                c.note(cx, "MoveToDestination", "stuck, no unstuck point -> FinishExecute(false)".into(), "BTTask_MoveToDestination_C");
                return FAILED;
            }
            return IN_PROGRESS;
        }
        if c.move_pending {
            return IN_PROGRESS;
        }
        if self.keep_moving(c, cx, p) {
            return self.set_path(c, cx, p);
        }
        SUCCEEDED
    }
}

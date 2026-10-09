//! The ranged motions (rust-combat r5): URangedDrawMotion, URangedReleaseMotion, UReloadMotion and URangedCancelMotion
//! (extract/native/decomp/URanged*Motion.cpp, UReloadMotion.cpp) with the equipment's fire request
//! (AMordhauEquipment::OnRequestFire_Implementation rva=0x155b3b0). The projectile itself is mh-character's
//! projectile.rs; the host takes `World::drain_fires` and spawns it (mh-sim ranged.rs), and its hits come back through
//! ranged.rs.
//!
//! - AMordhauCharacter::FirePressed rva=0x153d940 / FireReleased rva=0x153e890: bWantsFire. AMordhauCharacter::LODTick
//!   rva=0x154c390 (decomp 3669-3675): alive and bWantsFire -> RightHandEquipment (else Left)->OnRequestFire.
//! - OnRequestFire: bAllowFire (+0xcbc) and GetAmmo() != 0: not bIsLoaded (+0xd1f) -> bCanReload (+0xd1c) and
//!   CanInitiateMotion(UReloadMotion) and not (ReloadMovementRestriction (+0xccc) == 3 while falling) ->
//!   AssignNetMotion(type 0x11 Reload); loaded -> CanInitiateMotion(URangedDrawMotion) -> AssignNetMotion(type 0x0c Draw,
//!   param0 = a rand() byte (RandomValue), param1 = 1: the right hand).
//! - URangedDrawMotion ctor rva=0x1648fd0: bBlocksRegen, !bCanBlock, !bCanAttack, bDisablesAtmospherics,
//!   MovementRestriction 2. OnBegin rva=0x1661e50: bRangedActionAllowsRegen (+0x574) -> no regen block; SpeedFactor =
//!   RangedDrawSpeedFactor (+0xcd0); MovementRestriction = RangedDrawMovementRestriction (+0xccd); EndTime = FLT_MAX;
//!   DrawTime = StartTime + RangedDrawTime (+0xcec); SetTurnCaps(RangedDrawTurnCaps +0xcd8). OnTick rva=0x16694b0
//!   (decomp 138-160): bAllowHoldDraw (+0xcc6) and bDoNotFireAfterMaxHoldDrawTime (+0xce1) and now - StartTime >
//!   MaxHoldDrawTime (+0xcc8) and not fired -> bWantsFire = false, AssignNetMotion(type 0x12 Cancel).
//!   OnLateTick rva=0x1664d10: now > DrawTime -> SetAdditiveOverrideType("RangedDrawn", 3600) (animation side); not
//!   fired and (!bWantsFire or !bAllowHoldDraw or (now - StartTime > MaxHoldDrawTime and !bDoNotFireAfter...)) ->
//!   bHasFiredLocal, ServerAssignFireAim(CameraLocation1P, CameraRotation1P * AimRotationOffset, or the camera alone
//!   with bIsRangedSwayCameraBased +0x588), AssignNetMotion(type 0x0d Release). OnLeave rva=0x1665f60:
//!   SetTurnCaps(-1, -1), ResetAdditiveOverrideType. ProcessFeint rva=0x166be10: bAllowCancelDraw (+0xcc7) and not
//!   fired -> Cancel.
//! - URangedReleaseMotion ctor rva=0x1649030 (bBlocksRegen, !bCanAttack, !bCanBlock, MovementRestriction 2); OnBegin
//!   rva=0x16620d0: ReleaseDuration = max(RangedReleaseTime +0xcf4, 0.01); MovementRestriction =
//!   RangedReleaseMovementRestriction (+0xce0); EndTime = StartTime + ReleaseDuration; on the authority, the fire aim
//!   origin within 500 cm (250000 squared) of CameraLocation1P and (bIsLoaded or bFireThrowsEquipment) ->
//!   FireProjectile(LastRequestedFireOrigin, LastRequestedFireRotation); then the equipment's fired call (vcall
//!   +0x50, UNCONFIRMED: taken as Ammo - 1 and bIsLoaded = false when bCanReload).
//! - UReloadMotion ctor rva=0x16490f0 (bBlocksRegen, !bCanAttack, !bCanBlock); OnBegin rva=0x1662640: reload time
//!   T = max(RangedReloadTime +0xd00, 0.001); ReachTime = Start + T * RangedReloadGrabAmmoNormTime (+0xd0c),
//!   ReloadTime = Start + T * RangedReloadFinishReloadNormTime (+0xd10), EndTime = Start + T; MovementRestriction =
//!   ReloadMovementRestriction; SetTurnCaps(RangedReloadTurnCaps +0xd04). OnTick rva=0x1669f30: now > ReloadTime and
//!   !bIsLoaded and authority -> SetIsLoaded(true) (vcall 0xd9). OnLeave rva=0x16664c0: SetTurnCaps(-1, -1).
//! - URangedCancelMotion ctor rva=0x1648fa0; OnBegin rva=0x1661cc0: EndTime = StartTime + RangedCancelTime (+0xcf0)
//!   (StopAnim(RangedCancelTime): animation side).
//! - The draw sway / tremble (URangedDrawMotion::OnTick_Implementation rva=0x16694b0, decomp URangedDrawMotion.cpp 160-440), see
//!   `ranged_draw_sway`: AimRotationOffset (the fire aim, OnLateTick: Quat(CameraRotation1P) * Quat(AimRotationOffset)
//!   -> Rotator) and AimVisualRotationOffset (with the tremble; visual only).
//! Not ported (UNCONFIRMED): bIsRangedSwayCameraBased's camera branch (it feeds the delta angles to the controller
//! through vcalls 0xa10 / 0xa00; no stock equipment sets the flag), the ranger perk, quivers / restock, the equipment
//! montages, alternate ranged modes.

use super::motion::{Motion, MotionId, MotionKind};
use super::World;
use crate::data::{MotionBaseDef, MotionDef};
use crate::ue::FVector;
use serde_json::{json, Map, Value};
use std::rc::Rc;

/// a ranged equipment's data (AMordhauEquipment class defaults) and state
#[derive(Clone, Debug, PartialEq)]
pub struct RangedEquip {
    pub class: String,
    pub projectile_class: String,           // +0x648 ProjectileClass
    pub b_allow_fire: bool,                 // +0xcbc
    pub b_fire_throws_equipment: bool,      // +0xcbd
    pub b_allow_hold_draw: bool,            // +0xcc6
    pub b_allow_cancel_draw: bool,          // +0xcc7
    pub max_hold_draw_time: f64,            // +0xcc8
    pub reload_movement_restriction: i64,   // +0xccc
    pub draw_movement_restriction: i64,     // +0xccd
    pub draw_speed_factor: f64,             // +0xcd0
    pub draw_turn_caps: (f64, f64),         // +0xcd8
    pub release_movement_restriction: i64,  // +0xce0
    pub b_do_not_fire_after_max_hold: bool, // +0xce1
    pub draw_time: f64,                     // +0xcec
    pub cancel_time: f64,                   // +0xcf0
    pub release_time: f64,                  // +0xcf4
    pub reload_time: f64,                   // +0xd00
    pub reload_turn_caps: (f64, f64),       // +0xd04
    pub reload_grab_norm: f64,              // +0xd0c
    pub reload_finish_norm: f64,            // +0xd10
    pub b_can_reload: bool,                 // +0xd1c
    pub b_is_loaded: bool,                  // +0xd1f
    pub b_allows_regen: bool,               // +0x574 bRangedActionAllowsRegen
    pub b_sway_camera_based: bool,          // +0x588 bIsRangedSwayCameraBased
    pub ammo: i64,                          // +0x3e0
    /// RangedDrawSway (+0x580, a UCurveVector: the X / Y / Z FloatCurves as (keys, pre, post)); set by the host
    pub draw_sway: Option<Rc<[(Vec<crate::data::CurveKey>, String, String); 3]>>,
    pub airborne_sway: (f64, f64, f64),      // +0x58c RangedAirborneSway
    pub airborne_blend_in: f64,              // +0x598 RangedAirborneSwayBlendInSpeed
    pub airborne_blend_out: f64,             // +0x59c RangedAirborneSwayBlendOutSpeed
    pub sway_loop: (f64, f64),               // +0x5a0 RangedDrawSwayLoopSegment
    pub tremble_start_after: f64,            // +0x5a8 RangedDrawTremblingStartAfter
    pub tremble_max_after: f64,              // +0x5ac RangedDrawTremblingMaxAfter
    pub tremble_magnitude: f64,              // +0x5b0 RangedDrawTremblingMagnitude
    pub tremble_frequency: f64,              // +0x5b4 RangedDrawTremblingFrequency
}

impl RangedEquip {
    /// from the class defaults chain (a merged CDO object); absent values are the AMordhauEquipment ctor's
    /// (rva=0x1526480, decomp AMordhauEquipment.cpp 1712-1756: RangedDrawTime 0.5, RangedCancelTime 0.4,
    /// RangedReleaseTime 0.5, RangedReloadTime 0.6, GrabAmmoNormTime 0.4, FinishReloadNormTime 0.8, both TurnCaps
    /// (-1, -1), MaxHoldDrawTime 999999, ReloadMovementRestriction 1, RangedDrawMovementRestriction 2,
    /// RangedReleaseMovementRestriction 1, RangedDrawSpeedFactor 1), the rest the zero fill; Ammo absent = MaxAmmo
    pub fn from_defaults(class: &str, d: &Map<String, Value>) -> RangedEquip {
        let f = |k: &str, z: f64| crate::ue::f32r(d.get(k).and_then(|v| v.as_f64()).unwrap_or(z));
        let b = |k: &str| d.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
        // EMovementRestriction (PDB LF_ENUM; mh-character restriction): None 0, PartialSprint 1, Walk 2, NoMovement 3
        let mr = |k: &str, z: i64| match d.get(k) {
            Some(Value::String(s)) => match s.rsplit("::").next().unwrap_or(s) {
                "None" => 0,
                "PartialSprint" => 1,
                "Walk" => 2,
                "NoMovement" => 3,
                _ => z,
            },
            Some(v) => v.as_i64().unwrap_or(z),
            None => z,
        };
        let v2 = |k: &str| {
            d.get(k)
                .map(|v| (crate::ue::f32r(v["X"].as_f64().unwrap_or(0.0)), crate::ue::f32r(v["Y"].as_f64().unwrap_or(0.0))))
                .unwrap_or((-1.0, -1.0))
        };
        let proj = d.get("ProjectileClass").and_then(|v| v["ObjectPath"].as_str().or(v.as_str())).unwrap_or("").to_string();
        let proj = proj.split('.').next().unwrap_or("").to_string();
        let v3 = |k: &str| {
            let v = d.get(k).cloned().unwrap_or(Value::Null);
            let g = |c: &str| crate::ue::f32r(v[c].as_f64().unwrap_or(0.0));
            (g("X"), g("Y"), g("Z"))
        };
        let lp = v3("RangedDrawSwayLoopSegment");
        RangedEquip {
            class: class.to_string(),
            projectile_class: proj,
            b_allow_fire: b("bAllowFire"),
            b_fire_throws_equipment: b("bFireThrowsEquipment"),
            b_allow_hold_draw: b("bAllowHoldDraw"),
            b_allow_cancel_draw: b("bAllowCancelDraw"),
            max_hold_draw_time: f("MaxHoldDrawTime", 999999.0),
            reload_movement_restriction: mr("ReloadMovementRestriction", 1),
            draw_movement_restriction: mr("RangedDrawMovementRestriction", 2),
            draw_speed_factor: f("RangedDrawSpeedFactor", 1.0),
            draw_turn_caps: v2("RangedDrawTurnCaps"),
            release_movement_restriction: mr("RangedReleaseMovementRestriction", 1),
            b_do_not_fire_after_max_hold: b("bDoNotFireAfterMaxHoldDrawTime"),
            draw_time: f("RangedDrawTime", 0.5),
            cancel_time: f("RangedCancelTime", 0.4),
            release_time: f("RangedReleaseTime", 0.5),
            reload_time: f("RangedReloadTime", 0.6),
            reload_turn_caps: v2("RangedReloadTurnCaps"),
            reload_grab_norm: f("RangedReloadGrabAmmoNormTime", 0.4),
            reload_finish_norm: f("RangedReloadFinishReloadNormTime", 0.8),
            b_can_reload: b("bCanReload"),
            b_is_loaded: b("bIsLoaded"),
            b_allows_regen: b("bRangedActionAllowsRegen"),
            b_sway_camera_based: b("bIsRangedSwayCameraBased"),
            ammo: d.get("Ammo").and_then(|v| v.as_i64()).or_else(|| d.get("MaxAmmo").and_then(|v| v.as_i64())).unwrap_or(1),
            draw_sway: None,
            airborne_sway: v3("RangedAirborneSway"),
            airborne_blend_in: f("RangedAirborneSwayBlendInSpeed", 0.0),
            airborne_blend_out: f("RangedAirborneSwayBlendOutSpeed", 0.0),
            sway_loop: (lp.0, lp.1),
            tremble_start_after: f("RangedDrawTremblingStartAfter", 0.0),
            tremble_max_after: f("RangedDrawTremblingMaxAfter", 0.0),
            tremble_magnitude: f("RangedDrawTremblingMagnitude", 0.0),
            tremble_frequency: f("RangedDrawTremblingFrequency", 0.0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RangedStage {
    Draw,
    Release,
    Reload,
    Cancel,
}

/// the four ranged motions
#[derive(Clone, Debug)]
pub struct RangedMotion {
    pub stage: RangedStage,
    pub draw_time: f64,       // Draw: DrawTime
    pub has_fired: bool,      // Draw: bHasFiredLocal
    pub reach_time: f64,      // Reload
    pub reload_time: f64,     // Reload
    pub random_value: u8,     // Draw: RandomValue
    /// Draw: TrembleRotation (+0xa8), TrembleRotationStart (+0xb4), TrembleRotationTarget (+0xc0) as (pitch, yaw, roll)
    pub tremble: (f64, f64, f64),
    pub tremble_start: (f64, f64, f64),
    pub tremble_target: (f64, f64, f64),
    pub last_tremble_sample: f64, // +0xcc LastTimeTrembleRandomSampled
    pub airborne_blend: f64,      // +0x140 AirborneSwayBlend
    /// AimRotationOffset (+0xd4; the fire aim's offset) and AimVisualRotationOffset (+0xec), (pitch, yaw, roll)
    pub aim_offset: (f64, f64, f64),
    pub aim_visual: (f64, f64, f64),
}

/// a projectile the authority fires (FireProjectile): the host spawns `projectile_class` at `origin` / `rot`
#[derive(Clone, Debug, PartialEq)]
pub struct FireRequest {
    pub fighter: usize,
    pub projectile_class: String,
    pub origin: FVector,
    /// (pitch, yaw, roll) degrees
    pub rot: (f32, f32, f32),
}

pub fn ranged_def(stage: RangedStage) -> Rc<MotionDef> {
    let (rec, native, mr) = match stage {
        RangedStage::Draw => ("RangedDraw", "URangedDrawMotion", 2),
        RangedStage::Release => ("RangedRelease", "URangedReleaseMotion", 2),
        RangedStage::Reload => ("Reload", "UReloadMotion", 0),
        RangedStage::Cancel => ("RangedCancel", "URangedCancelMotion", 0),
    };
    Rc::new(MotionDef {
        base: MotionBaseDef {
            rec: rec.into(),
            path: String::new(),
            native: native.into(),
            b_is_flinchable: true,
            b_can_attack: false,
            b_can_block: false,
            b_can_emote: false,
            b_blocks_regen: true,
            speed_factor: 1.0,
            backpedal_speed_factor: 1.0,
            shield_wall_speed_factor: 1.0,
            movement_restriction: mr,
        },
        ..Default::default()
    })
}

impl World {
    pub fn ranged_m(&self, fi: usize, id: MotionId) -> Option<&RangedMotion> {
        if let MotionKind::Ranged(r) = &self.m(fi, id).k { Some(r) } else { None }
    }

    /// AMordhauCharacter::FirePressed / FireReleased
    pub fn set_wants_fire(&mut self, fi: usize, v: bool) {
        self.fighters[fi].wants_fire = v;
    }

    /// the fire requests made since the last call (host: spawn the projectiles)
    /// the current motion's (SpeedFactor, BackpedalSpeedFactor), as UMordhauMovementComponent::OnCharacterLODTick
    /// rva=0x14c9ec0 copies them into the movement each actor tick; None without a motion
    pub fn motion_speed_factors(&self, fi: usize) -> Option<(f64, f64)> {
        self.cur_m(fi).map(|m| (m.speed_factor, m.backpedal_speed_factor))
    }

    pub fn drain_fires(&mut self) -> Vec<FireRequest> {
        std::mem::take(&mut self.fires)
    }

    fn start_ranged(&mut self, fi: usize, stage: RangedStage, random: u8) -> MotionId {
        let id = self.alloc_motion(
            fi,
            Motion::new(ranged_def(stage), MotionKind::Ranged(Box::new(RangedMotion {
                stage,
                draw_time: 0.0,
                has_fired: false,
                reach_time: 0.0,
                reload_time: 0.0,
                random_value: random,
                tremble: (0.0, 0.0, 0.0),
                tremble_start: (0.0, 0.0, 0.0),
                tremble_target: (0.0, 0.0, 0.0),
                last_tremble_sample: 0.0,
                airborne_blend: 0.0,
                aim_offset: (0.0, 0.0, 0.0),
                aim_visual: (0.0, 0.0, 0.0),
            }))),
        );
        self.change(fi, id);
        id
    }

    /// UMotionSystemComponent::CanInitiateMotion for the ranged motions (UNCONFIRMED: the class rules are taken as
    /// UMordhauMotion's base: Idle motion or no current ranged motion)
    fn can_initiate_ranged(&self, fi: usize) -> bool {
        match self.cur_m(fi).map(|m| &m.k) {
            None | Some(MotionKind::Idle) => true,
            _ => false,
        }
    }

    /// AMordhauCharacter::LODTick's fire request (module docs)
    pub(crate) fn ranged_request_fire(&mut self, fi: usize) {
        let f = &self.fighters[fi];
        if f.dead || !f.wants_fire {
            return;
        }
        let Some(r) = f.ranged.clone() else { return };
        if !r.b_allow_fire || r.ammo == 0 {
            return;
        }
        if !r.b_is_loaded {
            let falling = self.fighters[fi].airborne;
            if r.b_can_reload && self.can_initiate_ranged(fi) && !(r.reload_movement_restriction == 3 && falling) {
                self.start_ranged(fi, RangedStage::Reload, 0);
            }
        } else if self.can_initiate_ranged(fi) {
            let rv = (self.rand.rand() & 0xff) as u8;
            self.start_ranged(fi, RangedStage::Draw, rv);
        }
    }

    pub(crate) fn ranged_on_begin(&mut self, fi: usize, id: MotionId) {
        let q = self.qf();
        let Some(r) = self.fighters[fi].ranged.clone() else {
            self.mm(fi, id).end_time = self.m(fi, id).start_time;
            return;
        };
        let start = self.m(fi, id).start_time;
        let stage = self.ranged_m(fi, id).map(|x| x.stage).unwrap();
        match stage {
            RangedStage::Draw => {
                let m = self.mm(fi, id);
                if r.b_allows_regen {
                    m.b_blocks_regen = false;
                }
                // URangedDrawMotion::OnBegin rva=0x1661e50: SpeedFactor = RangedDrawSpeedFactor (ranger perk: not ported)
                m.speed_factor = r.draw_speed_factor;
                m.movement_restriction = r.draw_movement_restriction;
                m.end_time = f32::MAX as f64;
                if let MotionKind::Ranged(x) = &mut m.k {
                    x.draw_time = q(r.draw_time + start);
                }
                self.set_turn_caps(fi, r.draw_turn_caps.0, r.draw_turn_caps.1);
            }
            RangedStage::Release => {
                let dur = if 0.01 < r.release_time { r.release_time } else { 0.01 };
                let m = self.mm(fi, id);
                if r.b_allows_regen {
                    m.b_blocks_regen = false;
                }
                m.movement_restriction = r.release_movement_restriction;
                m.end_time = q(dur + start);
                // the authority fires from the stored aim
                let (origin, rot) = self.fighters[fi].last_fire_aim;
                let cam = self.fighters[fi].geom.as_ref().map(|g| g.camera_loc).unwrap_or(origin);
                let d = cam - origin;
                if (d.x * d.x + d.y * d.y + d.z * d.z) < 250000.0 && (r.b_is_loaded || r.b_fire_throws_equipment) {
                    self.fires.push(FireRequest { fighter: fi, projectile_class: r.projectile_class.clone(), origin, rot });
                    let name = self.fighters[fi].name.clone();
                    self.emit_event(json!({"kind": "fire", "who": name, "projectile": r.projectile_class}));
                }
                let e = self.fighters[fi].ranged.as_mut().unwrap();
                e.ammo = (e.ammo - 1).max(0);
                if e.b_can_reload {
                    e.b_is_loaded = false;
                }
            }
            RangedStage::Reload => {
                let t = if 0.001 < r.reload_time { r.reload_time } else { 0.001 };
                let m = self.mm(fi, id);
                if r.b_allows_regen {
                    m.b_blocks_regen = false;
                }
                m.movement_restriction = r.reload_movement_restriction;
                m.end_time = q(start + t);
                if let MotionKind::Ranged(x) = &mut m.k {
                    x.reach_time = q(q(t * r.reload_grab_norm) + start);
                    x.reload_time = q(q(t * r.reload_finish_norm) + start);
                }
                self.set_turn_caps(fi, r.reload_turn_caps.0, r.reload_turn_caps.1);
            }
            RangedStage::Cancel => {
                self.mm(fi, id).end_time = q(r.cancel_time + start);
            }
        }
    }

    pub(crate) fn ranged_on_tick(&mut self, fi: usize, id: MotionId, dt: f64) {
        let Some(r) = self.fighters[fi].ranged.clone() else { return };
        let now = self.now;
        let start = self.m(fi, id).start_time;
        let Some(x) = self.ranged_m(fi, id).cloned() else { return };
        match x.stage {
            RangedStage::Draw => {
                if r.b_allow_hold_draw && r.b_do_not_fire_after_max_hold && !x.has_fired && r.max_hold_draw_time < now - start {
                    self.fighters[fi].wants_fire = false;
                    self.start_ranged(fi, RangedStage::Cancel, 0);
                    return;
                }
                self.ranged_draw_sway(fi, id, &r, dt);
            }
            RangedStage::Reload => {
                if x.reload_time < now && !r.b_is_loaded && self.authority {
                    self.fighters[fi].ranged.as_mut().unwrap().b_is_loaded = true;
                }
            }
            _ => {}
        }
    }

    /// URangedDrawMotion::OnTick_Implementation rva=0x16694b0 (decomp 160-440), the sway part:
    /// - tremble: with Frequency > 0 and Magnitude > 0 and StartAfter < T (T = now - StartTime - 0.2): every Frequency s
    ///   Start = Target, mag = GetNormalizedTime(StartAfter, MaxAfter, T) * Magnitude, r = rand()/32767,
    ///   Target.Yaw = (+-1) r mag, Target.Pitch = (+-1)(1 - |r|) mag (signs: int(rand() * 2/32767) == 1 -> +1);
    ///   Tremble = Start + normalized(Target - Start) * SmoothStep(0, 1, GetNormalizedTime(Last, Last + Frequency, now))
    /// - AirborneSwayBlend = FInterpTo(blend, falling ? 1 : 0, dt, falling ? BlendInSpeed : BlendOutSpeed)
    /// - the sway curve at T, wrapped into RangedDrawSwayLoopSegment (X, Y) when 0 < Y < T and X < Y:
    ///   T = (Y - X) * (fmod(T, Y) / Y) + X; Z negated when RandomValue & 1
    /// - AimRotationOffset = (blend * Airborne.Y + Curve.Y, blend * Airborne.Z * (RandomValue < 0x80 ? 1 : -1) + Curve.Z, 0)
    /// - AimVisualRotationOffset = (0, Aim.Yaw + Tremble.Yaw, -Aim.Pitch - Tremble.Pitch)
    fn ranged_draw_sway(&mut self, fi: usize, id: MotionId, r: &RangedEquip, dt: f64) {
        let q = self.qf();
        let now = self.now;
        let start = self.m(fi, id).start_time;
        let falling = self.fighters[fi].airborne;
        let Some(mut x) = self.ranged_m(fi, id).cloned() else { return };
        let norm_t = |s: f64, e: f64, c: f64| {
            if c < e {
                if c <= s {
                    return 0.0;
                }
                if s < e {
                    return q((c - s) / (e - s));
                }
            }
            1.0
        };
        let mut t = q(q(now - start) - 0.2);
        if 0.0 < r.tremble_frequency && 0.0 < r.tremble_magnitude && r.tremble_start_after < t {
            if q(r.tremble_frequency + x.last_tremble_sample) < now {
                x.tremble_start = x.tremble_target;
                x.last_tremble_sample = now;
                let mag = q(norm_t(r.tremble_start_after, r.tremble_max_after, t) * r.tremble_magnitude);
                let rr = q((self.rand.rand() & 0x7fff) as f64 * 3.051851e-05);
                let sign = |w: &mut World| if ((q((w.rand.rand() & 0x7fff) as f64 * 6.103702e-05)) as i64).min(1) == 1 { 1.0 } else { -1.0 };
                let s1 = sign(self);
                x.tremble_target.1 = q(q(s1 * rr) * mag);
                let s2 = sign(self);
                x.tremble_target.0 = q(q((1.0 - rr.abs()) * s2) * mag);
            }
            let a = norm_t(x.last_tremble_sample, q(x.last_tremble_sample + r.tremble_frequency), now);
            let a = if a < 0.0 { 0.0 } else if a >= 1.0 { 1.0 } else { q(a * a * (3.0 - 2.0 * a)) };
            // FRotator delta normalized to (-180, 180]
            let nd = |d: f64| {
                let mut v = d % 360.0;
                if v < 0.0 {
                    v += 360.0;
                }
                if v > 180.0 {
                    v -= 360.0;
                }
                v
            };
            let (s, g) = (x.tremble_start, x.tremble_target);
            x.tremble = (q(nd(g.0 - s.0) * a + s.0), q(nd(g.1 - s.1) * a + s.1), q(nd(g.2 - s.2) * a + s.2));
        }
        let (target, speed) = if falling { (1.0, r.airborne_blend_in) } else { (0.0, r.airborne_blend_out) };
        // FMath::FInterpTo
        x.airborne_blend = if speed <= 0.0 {
            target
        } else {
            let d = target - x.airborne_blend;
            if d * d < 1e-8 { target } else { q(x.airborne_blend + d * (dt * speed).clamp(0.0, 1.0)) }
        };
        let (mut cy, mut cz) = (0.0, 0.0);
        if let Some(c) = &r.draw_sway {
            let (lx, ly) = r.sway_loop;
            if 0.0 < ly && ly < t && lx < ly {
                t = q(q(ly - lx) * q(q(t % ly) / ly) + lx);
            }
            cy = self.spec.curve_eval(&c[1].0, t, &c[1].1, &c[1].2);
            cz = self.spec.curve_eval(&c[2].0, t, &c[2].1, &c[2].2);
            if x.random_value & 1 != 0 {
                cz = -cz;
            }
        }
        let sgn = if x.random_value < 0x80 { 1.0 } else { -1.0 };
        let b = x.airborne_blend;
        let aim = (q(b * r.airborne_sway.1 + cy), q(q(b * r.airborne_sway.2 * sgn) + cz), 0.0);
        x.aim_offset = aim;
        x.aim_visual = (0.0, q(aim.1 + x.tremble.1), q(-aim.0 - x.tremble.0));
        if let MotionKind::Ranged(m) = &mut self.mm(fi, id).k {
            **m = x;
        }
    }

    /// URangedDrawMotion::OnLateTick_Implementation (module docs), from the fighter's late tick
    pub(crate) fn ranged_late_tick(&mut self, fi: usize) {
        let Some(id) = self.fighters[fi].motion else { return };
        let Some(x) = self.ranged_m(fi, id).cloned() else { return };
        let Some(r) = self.fighters[fi].ranged.clone() else { return };
        let now = self.now;
        if x.stage != RangedStage::Draw || !(x.draw_time < now) || x.has_fired {
            return;
        }
        let start = self.m(fi, id).start_time;
        let wants = self.fighters[fi].wants_fire;
        if !wants || !r.b_allow_hold_draw || (r.max_hold_draw_time < now - start && !r.b_do_not_fire_after_max_hold) {
            if let MotionKind::Ranged(m) = &mut self.mm(fi, id).k {
                m.has_fired = true;
            }
            // ServerAssignFireAim(CameraLocation1P, (Quat(CameraRotation1P) * Quat(AimRotationOffset)).Rotator())
            // (OnLateTick decomp 105-142; bIsRangedSwayCameraBased: the camera rotation alone)
            let (loc, cam) = match self.fighters[fi].geom.as_ref() {
                Some(g) => (g.camera_loc, (g.camera_rot.0, g.camera_rot.1, 0.0f32)),
                None => (FVector::ZERO, (self.fighters[fi].look_up_value as f32, 0.0, 0.0)),
            };
            let ao = x.aim_offset;
            let rot = if r.b_sway_camera_based || ao == (0.0, 0.0, 0.0) {
                cam
            } else {
                let qa = crate::ue::FQuat::from_rotator(ao.0 as f32, ao.1 as f32, ao.2 as f32);
                let qc = crate::ue::FQuat::from_rotator(cam.0, cam.1, cam.2);
                super::geometry::quat_rotator(qc.mul(qa))
            };
            self.fighters[fi].last_fire_aim = (loc, rot);
            self.start_ranged(fi, RangedStage::Release, 0);
        }
    }

    pub(crate) fn ranged_on_leave(&mut self, fi: usize, id: MotionId) {
        if let Some(x) = self.ranged_m(fi, id) {
            if matches!(x.stage, RangedStage::Draw | RangedStage::Reload) {
                self.set_turn_caps(fi, -1.0, -1.0);
            }
        }
    }

    /// URangedDrawMotion::ProcessFeint_Implementation rva=0x166be10
    pub fn ranged_feint(&mut self, fi: usize) -> bool {
        let Some(id) = self.fighters[fi].motion else { return false };
        let Some(x) = self.ranged_m(fi, id).cloned() else { return false };
        let ok = x.stage == RangedStage::Draw && !x.has_fired && self.fighters[fi].ranged.as_ref().map(|r| r.b_allow_cancel_draw).unwrap_or(false);
        if ok {
            self.fighters[fi].wants_fire = false;
            self.start_ranged(fi, RangedStage::Cancel, 0);
        }
        ok
    }
}

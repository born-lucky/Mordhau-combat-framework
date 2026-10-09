//! The combat side of horses and riders (rust-combat r4). The horse's movement, gears, rearing and mounting geometry
//! are mh-character's exe_horse.rs; mh-sim (horse.rs there) owns the horse bodies and calls these rules.
//!
//! - UEnterVehicleMotion / ULeaveVehicleMotion (extract/native/decomp/UEnterVehicleMotion.cpp,
//!   ULeaveVehicleMotion.cpp): ctors rva=0x16467d0 / 0x1647030: bCanAttack / bCanBlock false (Leave:
//!   MovementRestriction 3); OnBegin_Implementation rva=0x165fb20 / 0x1660ee0: EndTime = StartTime + B * 0.1 + 0.25
//!   (Enter) / StartTime + B * 0.1 - 0.15 (Leave), B = the byte at the motion system's +0x26 (the net motion's
//!   first parameter as FNetMotion carries it: the host passes it, UNCONFIRMED field name), then StopAnim(0.5)
//!   (animation side). OnEnded: UMordhauMotion's (Idle). HorseState::driver_entering_or_leaving = the driver's
//!   current motion is one of these (AHorse::RequestRearing rva=0x1514b60).
//! - AHorse::DoKnockback rva=0x14f66e0 (AHorse.cpp 4-567), the trample: authority only; Other->LastKnockback + 0.2 >
//!   now -> false; RecentKnockbacks entries with time + 1 <= now (or dead) dropped; Other already in it -> false;
//!   impulse = the horse's RightVector * s * (|Velocity| * KnockbackForceVelocityFactor + KnockbackForce), s = +1 when
//!   Other is on the horse's right (InverseTransformPosition(Other).Y >= 0) else -1; Other->Knockback(impulse)
//!   (vcall +0x998, the movement side; false -> false). Accepted: RecentKnockbacks[Other] = now; damage =
//!   DamageFactorMultiplier * KnockbackDamage, times the Other's current UAttackMotion's RiposteTradeDamageFactor when
//!   it is a Riposte not in Recovery; flinch = the current motion bIsFlinchable and not a UParryMotion, and not
//!   friendly (AMordhauGameState::IsFriendly(horse, Other)); FMordhauDamageEvent on bone "Spine1" ->
//!   Other->TakeDamage (vcall +0x590); flinch and alive -> AssignNetMotion(FNetMotion::Flinched(CalculateDirection(
//!   Other - horse, Other's rotation), flag +0xa7a & 4, 1, 1)).
//! - AHorse::OnBumpCapsuleOverlapped rva=0x1507d80 (AHorse.cpp 1018-1088): the overlapped character not the horse's
//!   own recent rider (GetLastUsedVehicle(0.5)), horse alive with health; factor = BumpDamageBySpeedModifierCurve(
//!   |Velocity|) != 0 -> DoKnockback(Other, factor); true -> horse Velocity *= SpeedMultiplierOnBump (mh-character
//!   horse_bump_damage / horse_on_bump_knockback).
//! - AHorse::OnTookDamage_Implementation rva=0x1510210: alive and Melee -> Velocity *= SpeedMultiplierOnReceived-
//!   MeleeDamage (mh-character horse_on_melee_damage).
//! UNCONFIRMED: the damage type of the knockback's FMordhauDamageEvent (constructed without one: 0 here), the flinch
//! flag (+0xa7a bit 2: false here).

use super::enums::{at, stage};
use super::motion::{Motion, MotionId, MotionKind};
use super::World;
use crate::data::{MotionBaseDef, MotionDef};
use crate::ue::FVector;
use serde_json::json;
use std::rc::Rc;

/// UEnterVehicleMotion / ULeaveVehicleMotion
#[derive(Clone, Debug, Default)]
pub struct VehicleMotion {
    pub leaving: bool,
    /// the byte OnBegin reads (see the module docs)
    pub param: u8,
}

/// the native-only class defaults (UMordhauMotion ctor + the Enter / Leave ctors)
pub fn vehicle_def(leaving: bool) -> Rc<MotionDef> {
    Rc::new(MotionDef {
        base: MotionBaseDef {
            rec: if leaving { "LeaveVehicle" } else { "EnterVehicle" }.into(),
            path: String::new(),
            native: if leaving { "ULeaveVehicleMotion" } else { "UEnterVehicleMotion" }.into(),
            b_is_flinchable: true,
            b_can_attack: false,
            b_can_block: false,
            b_can_emote: false,
            b_blocks_regen: false,
            speed_factor: 1.0,
            backpedal_speed_factor: 1.0,
            shield_wall_speed_factor: 1.0,
            movement_restriction: if leaving { 3 } else { 0 },
        },
        ..Default::default()
    })
}

/// a rider's horse as ProcessHitForDamage reads it (host-fed each frame)
#[derive(Clone, Debug, Default)]
pub struct Mount {
    pub yaw: f32,
    pub velocity: FVector,
    /// AHorse AttackDamageBySpeedModifierCurve (+0xc20): keys, pre / post extrapolation; None = no curve
    pub attack_damage_curve: Option<(Vec<crate::data::CurveKey>, String, String)>,
}

/// The horse's knockback data (AHorse +0xc30 KnockbackForce, +0xc34 KnockbackForceVelocityFactor, +0xc38
/// KnockbackDamage; mh-character HorseCfg) and its RecentKnockbacks (TMap<actor, time>)
#[derive(Clone, Debug, Default)]
pub struct HorseCombat {
    pub knockback_force: f32,
    pub knockback_force_velocity_factor: f32,
    pub knockback_damage: f32,
    pub recent_knockbacks: Vec<(usize, f64)>,
}

/// DoKnockback's impulse: horse actor at `loc` facing `yaw` (degrees), velocity `vel`, the other at `other`
pub fn knockback_impulse(hc: &HorseCombat, loc: FVector, yaw: f32, vel: FVector, other: FVector) -> FVector {
    let q = crate::ue::FQuat::from_rotator(0.0, yaw, 0.0);
    let local = q.inverse().rotate(other - loc);
    let s = if local.y >= 0.0 { 1.0 } else { -1.0 };
    let right = q.rotate(FVector::new(0.0, 1.0, 0.0));
    let k = (vel.x * vel.x + vel.y * vel.y + vel.z * vel.z).sqrt() * hc.knockback_force_velocity_factor + hc.knockback_force;
    FVector::new(right.x * s * k, right.y * s * k, right.z * s * k)
}

impl World {
    /// a vehicle motion starts now (EnterVehicle / LeaveVehicle, offline AssignNetMotion shortcut as request_climb)
    pub fn request_vehicle_motion(&mut self, fi: usize, leaving: bool, param: u8) -> MotionId {
        let id = self.alloc_motion(fi, Motion::new(vehicle_def(leaving), MotionKind::Vehicle(Box::new(VehicleMotion { leaving, param }))));
        self.change(fi, id);
        id
    }

    /// U{Enter,Leave}VehicleMotion::OnBegin_Implementation (module docs), the combat field
    pub(crate) fn vehicle_on_begin(&mut self, fi: usize, id: MotionId) {
        let q = self.qf();
        let start = self.m(fi, id).start_time;
        let MotionKind::Vehicle(v) = &self.m(fi, id).k else { return };
        let b = q(v.param as f64 * q(0.1));
        let end = if v.leaving { q(q(b + start) - 0.15) } else { q(q(b + start) + 0.25) };
        self.mm(fi, id).end_time = end;
    }

    /// UAttackMotion::ProcessHitForDamage rva=0x1638a60 (decomp UAttackMotion.cpp 264-395): with the attacker riding a
    /// horse, the damage factor (GlobalDamageModifier) is replaced by AttackDamageBySpeedModifierCurve(X of the horse
    /// rotation's inverse applied to (horse velocity - defender velocity)); else with the defender riding and the
    /// bone not a leg (IsLeg), the defender horse's curve on (defender horse velocity - attacker velocity) in its
    /// frame. No curve -> unchanged. The defender's velocity is its horse's when it rides.
    pub fn mounted_damage_factor(&self, a: usize, v: usize, bone: &str) -> Option<f64> {
        let fa = &self.fighters[a];
        let fv = &self.fighters[v];
        let vel = |f: &super::system::Fighter| f.mount.as_ref().map(|m| m.velocity).unwrap_or(f.velocity);
        let (m, mine, other) = if let Some(m) = &fa.mount {
            (m, vel(fa), vel(fv))
        } else if let Some(m) = fv.mount.as_ref().filter(|_| !super::damage::is_leg(&self.spec.constants, bone)) {
            (m, vel(fv), vel(fa))
        } else {
            return None;
        };
        let (keys, pre, post) = m.attack_damage_curve.as_ref()?;
        let q = crate::ue::FQuat::from_rotator(0.0, m.yaw, 0.0);
        let local = q.inverse().rotate(mine - other);
        Some(self.spec.curve_eval(keys, local.x as f64, pre, post))
    }

    /// the driver's current motion is an Enter / Leave vehicle motion (HorseState::driver_entering_or_leaving)
    pub fn entering_or_leaving_vehicle(&self, fi: usize) -> bool {
        matches!(self.cur_m(fi).map(|m| &m.k), Some(MotionKind::Vehicle(_)))
    }

    /// AHorse::DoKnockback rva=0x14f66e0 after the impulse was computed and accepted by the other's movement
    /// (`knockback_accepted`), see the module docs. `horse_driver`: the horse's driver fighter (the horse's team for
    /// IsFriendly; None = no team); `other_last_knockback`: Other->LastKnockback; `direction`: CalculateDirection(Other
    /// - horse, Other's rotation) (the host's, mh-character calculate_direction). Returns whether it knocked back.
    #[allow(clippy::too_many_arguments)]
    pub fn horse_do_knockback(&mut self, hc: &mut HorseCombat, horse_driver: Option<usize>, other: usize, damage_factor: f32, other_last_knockback: f64, direction: f64, knockback_accepted: bool) -> bool {
        let now = self.now;
        if now < other_last_knockback + 0.2 {
            return false;
        }
        let dead: Vec<bool> = self.fighters.iter().map(|f| f.dead).collect();
        hc.recent_knockbacks.retain(|(o, t)| !(t + 1.0 <= now) && !dead.get(*o).copied().unwrap_or(true));
        if hc.recent_knockbacks.iter().any(|(o, _)| *o == other) {
            return false;
        }
        if !knockback_accepted {
            return false;
        }
        hc.recent_knockbacks.push((other, now));
        let q = self.qf();
        let mut dmg = q(damage_factor as f64 * hc.knockback_damage as f64);
        let mut flinch = false;
        if let Some(m) = self.cur_m(other) {
            flinch = m.b_is_flinchable && !m.is_parry();
            if let Some(a) = m.attack() {
                if a.ty == at::RIPOSTE && a.stage != stage::RECOVERY {
                    dmg = q(dmg * m.def.attack().riposte_trade_damage_factor);
                }
            }
        }
        if horse_driver.is_some() && self.is_friendly(horse_driver, Some(other)) {
            flinch = false;
        }
        let applied = self.take_damage(other, dmg, horse_driver, 0);
        let vn = self.fighters[other].name.clone();
        self.trace_event(&format!("horse trampled {vn} {dmg:.2}"));
        let health = self.fighters[other].health;
        self.emit_event(json!({"kind": "hit", "horse": true, "victim": vn, "bone": "Spine1", "damage": dmg, "applied": applied, "health": health}));
        if flinch && !self.fighters[other].dead {
            self.assign_net_flinched(other, direction, false, 1.0, 1.0);
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn impulse_side_and_magnitude() {
        let hc = HorseCombat { knockback_force: 100.0, knockback_force_velocity_factor: 0.5, knockback_damage: 10.0, recent_knockbacks: vec![] };
        // horse facing +X at the origin moving 200 cm/s: someone at +Y (its right) is pushed +Y with 100 + 0.5 * 200
        let i = knockback_impulse(&hc, FVector::ZERO, 0.0, FVector::new(200.0, 0.0, 0.0), FVector::new(50.0, 80.0, 0.0));
        assert!((i.y - 200.0).abs() < 1e-3 && i.x.abs() < 1e-3);
        let j = knockback_impulse(&hc, FVector::ZERO, 0.0, FVector::new(200.0, 0.0, 0.0), FVector::new(50.0, -80.0, 0.0));
        assert!((j.y + 200.0).abs() < 1e-3);
    }
}

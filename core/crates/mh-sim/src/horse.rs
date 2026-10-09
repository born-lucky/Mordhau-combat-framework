//! Horses in the Sim (rust-combat r4): mh-character's exe_horse.rs moves them (UHorseMovementComponent, gears, turning,
//! rearing, mounting geometry); mordhau-core's combat/horse.rs has the trample (AHorse::DoKnockback) and the
//! Enter / Leave vehicle motions. This module owns the horse bodies and wires the two:
//!
//! - mount (UMordhauVehicleComponent::StartDriving rva=0x14d79a0, decomp UMordhauVehicleComponent.cpp 4-388):
//!   CanInteract (horse_can_interact) -> UEnterVehicleMotion; the rider's equipment (decomp 86-132): with
//!   bDisarmOnEnter false (BP_VehicleHorse) a right hand item that cannot be used on a horse (bCanEquipOnHorse +0xc71
//!   false) switches mode when its alternate mode can (bHasAlternateMode, bSecondCanEquipOnHorse +0xc73), else it is
//!   holstered (StoredRightHandSlot = its index; AMordhauCharacter::Holster) - here: the fists are equipped (the
//!   `switch_to_fists` answer) and its actor token is kept; the left hand is not modelled (UNCONFIRMED). Then the controller
//!   drives the horse and the rider is attached at the seat (horse_rider_seat) every frame.
//! - dismount (StopDriving rva=0x14d8270, decomp 733-962): ULeaveVehicleMotion; the exit spot (horse_stop_driving);
//!   the stored slot re-equipped (SwitchEquipmentByIndex(StoredRightHandSlot), decomp 945-953).
//! - every frame: the driver's input drives the horse (it is the possessed pawn); HorseState::driver_entering_or_leaving
//!   = the driver's motion is an Enter / Leave vehicle motion; AHorse::LODTick's regen gates (horse_regen_flags:
//!   !bCanRiderRegenStamina -> StopStaminaRegen on the driver; health regen is not modelled in the core); the
//!   BumpCollider overlap (AHorse::OnBumpCapsuleOverlapped rva=0x1507d80) -> DoKnockback on the character it touches,
//!   not the horse's own rider of the last 0.5 s (GetLastUsedVehicle(0.5)).
//! UNCONFIRMED: the BumpCollider's attach parent (taken as the actor root: BP_Horse BumpCollider_GEN_VARIABLE
//! RelativeLocation / Rotation / Scale3D), the character capsule (34 / 88), AMordhauCharacter::Knockback's movement
//! (a velocity add), the horse taking melee hits (horses are not trace targets yet: `horse_took_melee` is the hook),
//! the mounted attack angle and the per-weapon horseback attack / block / couch gates.

use crate::sim::{Sim, SimInput};
use mh_character::exe::ExeMovement;
use mh_character::exe_horse::{horse_character_records, HorseCfg, HorseInput};
use mh_character::CharacterRecords;
use mordhau_core::combat::horse::{knockback_impulse, HorseCombat};
use mordhau_core::combat::EquipmentId;
use mordhau_core::ue::{FQuat, FTransform, FVector};

pub const BP_HORSE: &str = "Mordhau/Content/Mordhau/Blueprints/Interactables/Animals/BP_Horse";
pub const BP_VEHICLE_HORSE: &str = "Mordhau/Content/Mordhau/Blueprints/VehicleComponents/BP_VehicleHorse";
/// UMordhauCharacter's CollisionCylinder (BP_MordhauCharacter; UNCONFIRMED here, the mh-character default)
const CHAR_RADIUS: f32 = 34.0;
const CHAR_HALF_HEIGHT: f32 = 88.0;

/// BP_Horse BumpCollider: relative location, pitch, radius and half height (scaled)
#[derive(Clone, Copy, Debug)]
pub struct Bump {
    pub rel: FVector,
    pub pitch: f32,
    pub radius: f32,
    pub half_height: f32,
}

pub struct SimHorse {
    pub m: ExeMovement,
    pub combat: HorseCombat,
    pub bump: Bump,
    pub rider: Option<usize>,
    /// the rider's holstered right hand weapon (StoredRightHandSlot)
    pub stored_right: Option<StoredRight>,
    /// (fighter, time it last left this horse) for GetLastUsedVehicle(0.5)
    pub last_used: Vec<(usize, f64)>,
    pub input: HorseInput,
    /// AHorse AttackDamageBySpeedModifierCurve (+0xc20; mordhau-core horse.rs mounted_damage_factor)
    pub attack_damage_curve: Option<(Vec<mordhau_core::data::CurveKey>, String, String)>,
    /// the horse's health (AAdvancedCharacter Health; UNCONFIRMED start value 100) and the attacks that hit it
    pub health: f64,
    pub hit_by: Vec<(usize, f64)>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoredRight { pub actor: EquipmentId, pub pawn_id: u32 }

/// a horse from the paks (BP_Horse + BP_VehicleHorse + the curves), `base` = the character records the horse
/// records start from (horse_character_records)
pub fn load_horse(rd: &mh_pak::Reader, base: &CharacterRecords, location: FVector, yaw: f32) -> Result<SimHorse, String> {
    let js = |p: &str| rd.read(p).map(|ex| serde_json::Value::Array(ex).to_string());
    let bp = js(BP_HORSE).ok_or("no BP_Horse in the paks")?;
    let veh = js(BP_VEHICLE_HORSE);
    let curve = |p: &str| js(p);
    let cfg = HorseCfg::from_json(&bp, veh.as_deref(), &curve)?;
    let rec = horse_character_records(base, &bp)?;
    let mut m = ExeMovement::new_horse(&rec, &bp, cfg.clone(), location)?;
    m.yaw = yaw;
    let ex: serde_json::Value = serde_json::from_str(&bp).map_err(|e| e.to_string())?;
    let named = |n: &str| ex.as_array().and_then(|a| a.iter().find(|e| e["Name"].as_str().map(|s| s.starts_with(n)).unwrap_or(false))).map(|e| e["Properties"].clone());
    let b = named("BumpCollider").unwrap_or(serde_json::Value::Null);
    let g = |o: &serde_json::Value, k: &str, d: f64| o[k].as_f64().unwrap_or(d) as f32;
    let (rl, rr, rs) = (&b["RelativeLocation"], &b["RelativeRotation"], &b["RelativeScale3D"]);
    let sxy = g(rs, "X", 1.0).max(g(rs, "Y", 1.0));
    let bump = Bump {
        rel: FVector::new(g(rl, "X", 0.0), g(rl, "Y", 0.0), g(rl, "Z", 0.0)),
        pitch: g(rr, "Pitch", 0.0),
        radius: g(&b, "CapsuleRadius", 22.0) * sxy,
        half_height: g(&b, "CapsuleHalfHeight", 22.0) * g(rs, "Z", 1.0),
    };
    let cdo = rd.defaults(BP_HORSE);
    let f = |k: &str| cdo.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
    let combat = HorseCombat {
        knockback_force: f("KnockbackForce"),
        knockback_force_velocity_factor: f("KnockbackForceVelocityFactor"),
        knockback_damage: f("KnockbackDamage"),
        recent_knockbacks: Vec::new(),
    };
    let _ = cfg;
    let curve = |k: &str| -> Option<(Vec<mordhau_core::data::CurveKey>, String, String)> {
        let p = cdo.get(k)?["ObjectPath"].as_str()?.split('.').next()?.to_string();
        let ex = rd.read(&p)?;
        let e = mh_pak::pkg::export_of(&ex, "CurveFloat")?;
        let fc = &e["Properties"]["FloatCurve"];
        let keys = serde_json::from_value(fc["Keys"].clone()).ok()?;
        let tail = |v: &serde_json::Value| v.as_str().map(|s| s.rsplit("::").next().unwrap_or(s).to_string()).unwrap_or_else(|| "RCCE_Constant".into());
        Some((keys, tail(&fc["PreInfinityExtrap"]), tail(&fc["PostInfinityExtrap"])))
    };
    let attack_damage_curve = curve("AttackDamageBySpeedModifierCurve");
    Ok(SimHorse { m, combat, bump, rider: None, stored_right: None, last_used: Vec::new(), input: HorseInput::default(), attack_damage_curve, health: 100.0, hit_by: Vec::new() })
}

/// AMordhauEquipment bCanEquipOnHorse / bSecondCanEquipOnHorse / bHasAlternateMode from the class defaults (absent:
/// false, the zero-filled ctor)
pub fn equip_on_horse_flags(rd: &mh_pak::Reader, weapon: &str) -> (bool, bool, bool) {
    let d = rd.defaults(weapon);
    let b = |k: &str| d.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
    (b("bCanEquipOnHorse"), b("bSecondCanEquipOnHorse"), b("bHasAlternateMode"))
}

/// closest distance between segments [p1, q1] and [p2, q2]
fn seg_seg_dist(p1: FVector, q1: FVector, p2: FVector, q2: FVector) -> f32 {
    let (d1, d2, r) = (q1 - p1, q2 - p2, p1 - p2);
    let (a, e, f) = (d1.dot(d1), d2.dot(d2), d2.dot(r));
    let (mut s, mut t);
    if a <= 1e-8 && e <= 1e-8 {
        return r.length();
    }
    if a <= 1e-8 {
        s = 0.0;
        t = (f / e).clamp(0.0, 1.0);
    } else {
        let c = d1.dot(r);
        if e <= 1e-8 {
            t = 0.0;
            s = (-c / a).clamp(0.0, 1.0);
        } else {
            let b = d1.dot(d2);
            let den = a * e - b * b;
            s = if den != 0.0 { ((b * f - c * e) / den).clamp(0.0, 1.0) } else { 0.0 };
            t = (b * s + f) / e;
            if t < 0.0 {
                t = 0.0;
                s = (-c / a).clamp(0.0, 1.0);
            } else if t > 1.0 {
                t = 1.0;
                s = ((b - c) / a).clamp(0.0, 1.0);
            }
        }
    }
    let c1 = p1 + FVector::new(d1.x * s, d1.y * s, d1.z * s);
    let c2 = p2 + FVector::new(d2.x * t, d2.y * t, d2.z * t);
    (c1 - c2).length()
}

impl Sim {
    pub fn add_horse(&mut self, h: SimHorse) -> usize {
        self.horses.push(h);
        self.horses.len() - 1
    }

    /// the horse fighter `fi` rides
    pub fn riding(&self, fi: usize) -> Option<usize> {
        self.horses.iter().position(|h| h.rider == Some(fi))
    }

    /// StartDriving (module docs); `equip`: equip_on_horse_flags of the rider's right hand weapon; `fists`: the
    /// weapon equipped when it is holstered. Returns whether the rider mounted.
    pub fn mount(&mut self, fi: usize, hi: usize, equip: (bool, bool, bool), fists: &str) -> bool {
        if self.riding(fi).is_some() || self.combat.fighters[fi].dead {
            return false;
        }
        let rider = self.movers[fi].location;
        if !self.horses[hi].m.horse_can_interact(rider, true) {
            return false;
        }
        // UEnterVehicleMotion (param 0: UNCONFIRMED enter info byte)
        self.combat.request_vehicle_motion(fi, false, 0);
        let (can, second, alt) = equip;
        if !can {
            if alt && second {
                // AMordhauCharacter::SwitchModeAndReAttach (the direct swap, no UEquipmentModeSwitchMotion; fp-anim r3)
                self.combat.switch_mode_and_reattach(fi);
            } else {
                let fists_actor = if fists.is_empty() { None } else {
                    match self.combat.designated_fists_actor(fi, fists) {
                        Ok(id) => Some(id),
                        Err(error) => { self.combat.emit_event(serde_json::json!({"kind":"equipment_error", "operation":"horse_fists", "error":error})); return false; }
                    }
                };
                if let Some(actor) = self.combat.fighters[fi].right_actor {
                    let pawn_id = self.combat.fighters[fi].id;
                    if let Err(error) = self.combat.holster_actor(fi, actor) {
                        self.combat.emit_event(serde_json::json!({"kind":"equipment_error", "operation":"horse_holster", "error":error})); return false;
                    }
                    self.horses[hi].stored_right = Some(StoredRight { actor, pawn_id });
                }
                if let Some(actor) = fists_actor {
                    if let Err(error) = self.combat.equip_right_actor(fi, actor) {
                        self.combat.emit_event(serde_json::json!({"kind":"equipment_error", "operation":"horse_fists", "error":error})); return false;
                    }
                }
                let name = self.combat.fighters[fi].name.clone();
                self.combat.emit_event(serde_json::json!({"kind": "holster", "who": name}));
            }
        }
        let Some((seat, yaw)) = self.horses[hi].m.horse_start_driving(fi) else { return false };
        self.horses[hi].rider = Some(fi);
        self.combat.fighters[fi].mount = Some(self.mount_of(hi));
        self.movers[fi].location = seat;
        self.yaw[fi] = yaw;
        true
    }

    /// StopDriving (module docs); `exit`: the mesh's Exit socket location when the host knows it
    pub fn dismount(&mut self, fi: usize, exit: Option<FVector>) -> bool {
        let Some(hi) = self.riding(fi) else { return false };
        self.combat.request_vehicle_motion(fi, true, 0);
        let (loc, yaw) = {
            let rl = self.movers[fi].location;
            let ry = self.yaw[fi];
            self.horses[hi].m.horse_stop_driving(self.collision.as_ref(), rl, ry, exit, CHAR_RADIUS, CHAR_HALF_HEIGHT)
        };
        self.movers[fi].location = loc;
        self.yaw[fi] = yaw;
        let now = self.combat.now;
        self.combat.fighters[fi].mount = None;
        let h = &mut self.horses[hi];
        h.rider = None;
        h.last_used.retain(|x| x.0 != fi);
        h.last_used.push((fi, now));
        if let Some(stored) = h.stored_right.take() {
            let result = if self.combat.fighters[fi].id == stored.pawn_id {
                self.combat.equip_right_actor(fi, stored.actor)
            } else { Err("stored horse equipment belongs to a previous pawn".to_string()) };
            if let Err(error) = result {
                self.combat.emit_event(serde_json::json!({"kind":"equipment_error", "operation":"horse_restore", "error":error}));
            }
        }
        true
    }

    /// Before a pawn is replaced/removed: clear exact saved references and update index-based horse lists.
    /// This releases membership only; native equipment destruction remains an explicit host boundary.
    pub(crate) fn release_horse_pawn(&mut self, fi: usize, removing: bool) {
        let pawn_id = self.combat.fighters[fi].id;
        for h in &mut self.horses {
            if h.stored_right.is_some_and(|stored| stored.pawn_id == pawn_id) { h.stored_right = None; }
            if h.rider == Some(fi) { h.rider = None; h.input = HorseInput::default(); }
            if let Some(hs) = &mut h.m.horse {
                if hs.driver == Some(fi) { hs.driver = None; hs.controlled = false; hs.driver_entering_or_leaving = false; }
                else if removing { hs.driver = hs.driver.map(|i| if i > fi { i - 1 } else { i }); }
            }
            if removing { h.rider = h.rider.map(|i| if i > fi { i - 1 } else { i }); }
            for entries in [&mut h.last_used, &mut h.hit_by, &mut h.combat.recent_knockbacks] {
                entries.retain(|(i, _)| *i != fi);
                if removing { for (i, _) in entries { if *i > fi { *i -= 1; } } }
            }
        }
    }

    /// the rider's Mount as mordhau-core reads it
    fn mount_of(&self, hi: usize) -> mordhau_core::combat::horse::Mount {
        let h = &self.horses[hi];
        mordhau_core::combat::horse::Mount { yaw: h.m.yaw, velocity: h.m.velocity, attack_damage_curve: h.attack_damage_curve.clone() }
    }

    /// AHorse::OnTookDamage_Implementation's melee branch: alive and melee -> Velocity *= SpeedMultiplierOnReceived-
    /// MeleeDamage (horse_on_melee_damage); the health takes `damage`
    pub fn horse_took_melee(&mut self, hi: usize, damage: f64) {
        let h = &mut self.horses[hi];
        if h.m.dead {
            return;
        }
        h.m.horse_on_melee_damage();
        h.health = (h.health - damage).max(0.0);
        if h.health <= 0.0 {
            h.m.dead = true;
        }
    }

    /// Horses as melee trace targets (UNCONFIRMED shape: the horse's CollisionCylinder capsule stands in for its
    /// physics asset, which is not loaded): each attacker in Release sweeps its current weapon segment
    /// (CurrentTraceStart -> CurrentTraceEnd) against every horse it does not ride; the first contact of an attack
    /// hits the horse once (ProcessHitForDamage's ActorSetCache rule) with ComputeMeleeDamage at armour tier 0 on
    /// "Spine1" (UNCONFIRMED: the horse's armour / bone mapping), then horse_took_melee
    fn horse_traces(&mut self) {
        let caps: Vec<(usize, FVector, f32, f32)> = self.horses.iter().enumerate().filter(|(_, h)| !h.m.dead).map(|(i, h)| (i, h.m.location, h.m.e.capsule_radius, h.m.e.capsule_half_height)).collect();
        if caps.is_empty() {
            return;
        }
        for fi in 0..self.combat.fighters.len() {
            let Some(m) = self.combat.cur_m(fi) else { continue };
            let Some(at) = m.attack() else { continue };
            if at.stage != 1 {
                continue;
            }
            let (st, ai) = (m.start_time, at.ai.clone());
            let name = self.combat.fighters[fi].name.clone();
            let (a, b) = match self.posed.borrow().get(&name) {
                Some(p) if p.tracer.cur_valid => (p.tracer.cur_start, p.tracer.cur_end),
                _ => continue,
            };
            for &(hi, loc, r, hh) in &caps {
                if self.horses[hi].rider == Some(fi) || self.horses[hi].hit_by.iter().any(|x| *x == (fi, st)) {
                    continue;
                }
                let shape = crate::physics::Shape::Capsule { radius: r, half_len: (hh - r).max(0.0) };
                let xf = FTransform::new(FQuat::IDENTITY, loc);
                if crate::physics::segment_shape(a, b, &xf, &shape).is_some() {
                    self.horses[hi].hit_by.push((fi, st));
                    let d = ai.damage.first().copied().unwrap_or(0.0) as f64;
                    self.horse_took_melee(hi, d);
                    self.combat.emit_event(serde_json::json!({"kind": "horse_hit", "attacker": name, "horse": hi, "damage": d}));
                }
            }
        }
    }

    /// the driver's movement input becomes the horse's (called by step before the movement)
    pub(crate) fn horse_inputs(&mut self, inputs: &[(usize, SimInput)]) {
        for h in self.horses.iter_mut() {
            let Some(r) = h.rider else { continue };
            let i = inputs.iter().find(|x| x.0 == r).map(|x| x.1.clone()).unwrap_or_default();
            h.input = HorseInput { fwd: i.fwd, right: i.right, jump: i.jump };
        }
    }

    /// one frame of every horse (module docs), after the late tick
    pub(crate) fn tick_horses(&mut self) {
        if self.horses.is_empty() {
            return;
        }
        let dt = self.dt;
        let now = self.combat.now;
        for hi in 0..self.horses.len() {
            let rider = self.horses[hi].rider;
            let entering = rider.map(|r| self.combat.entering_or_leaving_vehicle(r)).unwrap_or(false);
            {
                let h = &mut self.horses[hi];
                if let Some(hs) = h.m.horse.as_deref_mut() {
                    hs.driver_entering_or_leaving = entering;
                    hs.controlled = rider.is_some();
                }
                h.m.world_time = now as f32;
                let inp = h.input;
                h.m.horse_frame(self.collision.as_ref(), dt, &inp);
            }
            // the rider rides at the seat; the regen gates of the current gear
            if let Some(r) = rider {
                let (seat, yaw) = self.horses[hi].m.horse_rider_seat();
                self.movers[r].location = seat;
                self.movers[r].velocity = self.horses[hi].m.velocity;
                self.yaw[r] = yaw;
                if let Some(g) = self.horses[hi].m.horse_regen_flags() {
                    if !g.can_rider_regen_stamina {
                        self.combat.stop_stamina_regen(r, 0.0);
                    }
                }
            }
            self.horse_bumps(hi);
            if let Some(r) = rider {
                self.combat.fighters[r].mount = Some(self.mount_of(hi));
            }
        }
        self.horse_traces();
    }

    /// AHorse::OnBumpCapsuleOverlapped against every other character this frame
    fn horse_bumps(&mut self, hi: usize) {
        let now = self.combat.now;
        let (loc, yaw, vel) = (self.horses[hi].m.location, self.horses[hi].m.yaw, self.horses[hi].m.velocity);
        if self.horses[hi].m.dead {
            return;
        }
        let b = self.horses[hi].bump;
        let q = FQuat::from_rotator(0.0, yaw, 0.0);
        let centre = q.rotate(b.rel) + loc;
        let axis = q.mul(FQuat::from_rotator(b.pitch, 0.0, 0.0)).rotate(FVector::new(0.0, 0.0, (b.half_height - b.radius).max(0.0)));
        let (p1, q1) = (centre - axis, centre + axis);
        for fi in 0..self.combat.fighters.len() {
            if self.combat.fighters[fi].dead || self.horses[hi].rider == Some(fi) {
                continue;
            }
            if self.horses[hi].last_used.iter().any(|x| x.0 == fi && now - x.1 <= 0.5) {
                continue;
            }
            let c = self.movers[fi].location;
            let up = FVector::new(0.0, 0.0, CHAR_HALF_HEIGHT - CHAR_RADIUS);
            if seg_seg_dist(p1, q1, c - up, c + up) > b.radius + CHAR_RADIUS {
                continue;
            }
            let factor = self.horses[hi].m.horse_bump_damage(vel);
            if factor == 0.0 {
                continue;
            }
            let imp = knockback_impulse(&self.horses[hi].combat, loc, yaw, vel, c);
            while self.last_knockback.len() <= fi {
                self.last_knockback.push(f64::NEG_INFINITY);
            }
            let dir = mh_character::uequat::calculate_direction(c - loc, 0.0, self.yaw[fi], 0.0) as f64;
            let driver = self.horses[hi].rider;
            let last = self.last_knockback[fi];
            let mut hc = std::mem::take(&mut self.horses[hi].combat);
            let done = self.combat.horse_do_knockback(&mut hc, driver, fi, factor, last, dir, true);
            self.horses[hi].combat = hc;
            if done {
                // AMordhauCharacter::Knockback: LastKnockback = now, the launch (UNCONFIRMED: a velocity add)
                self.last_knockback[fi] = now;
                let m = &mut self.movers[fi];
                m.velocity = m.velocity + imp;
                self.horses[hi].m.horse_on_bump_knockback();
            }
        }
    }
}

#[cfg(test)]
mod equipment_tests {
    use super::*;
    use crate::FighterDesc;
    use mh_character::{BoxWorld, CharacterSource, RecordsJson};
    use mordhau_core::combat::EquipmentPlacement;
    use std::{path::PathBuf, rc::Rc, sync::Arc};

    const LS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";
    const FISTS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/BP_FistsWeapon";
    fn fixture() -> Option<(Sim, usize)> {
        let required = std::env::var("MORDHAU_GOLDEN_REQUIRED").is_ok_and(|v| v == "1");
        let setup = || -> Result<(Sim, usize), String> {
            let matrix = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false).map_err(|e| format!("matrix: {e:?}"))?;
            let vfs = Arc::new(mh_pak::Vfs::mount_default().map_err(|e| format!("paks: {e:?}"))?);
            let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json");
            let txt = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
            let records = RecordsJson(&txt).load().map_err(|e| format!("character records: {e:?}"))?;
            let ld = crate::load::load(&matrix, vfs.clone(), &[LS, FISTS]).map_err(|e| format!("load: {e:?}"))?;
            let mut floor = BoxWorld::new();
            floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
            let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), records, Box::new(floor), 1.0/60.0);
            let rd = mh_pak::Reader::new(vfs);
            let horse = load_horse(&rd, &s.records, FVector::new(0.0, 0.0, 125.0), 0.0)?;
            let hi = s.add_horse(horse);
            Ok((s, hi))
        };
        match setup() { Ok(v) => Some(v), Err(error) => { assert!(!required, "required original horse fixture: {error}"); eprintln!("SKIP: {error}"); None } }
    }
    fn rider(s: &mut Sim, hi: usize, name: &str) -> usize {
        let (location, yaw) = s.horses[hi].m.horse_rider_seat();
        s.add_fighter(&FighterDesc { name: name.into(), weapon: LS.into(), location, yaw, ..Default::default() })
    }

    #[test]
    fn equipment_horse_holster_and_restore_preserve_actor_mode() {
        let Some((mut s, hi)) = fixture() else { return };
        let fi = rider(&mut s, hi, "A");
        let actor = s.combat.fighters[fi].right_actor.unwrap();
        s.combat.switch_mode_and_reattach(fi);
        let data = s.combat.equipment_actor(actor).unwrap().weapon.clone().unwrap();
        // Select the recovered holster branch explicitly; this is a lifecycle fixture, not a claim about LS's flags.
        assert!(s.mount(fi, hi, (false, false, false), FISTS));
        assert_eq!(s.horses[hi].stored_right, Some(StoredRight { actor, pawn_id: s.combat.fighters[fi].id }));
        assert_eq!(s.combat.equipment_actor(actor).unwrap().placement, EquipmentPlacement::Holstered);
        let fists = s.combat.fighters[fi].right_actor.unwrap();
        assert_ne!(actor, fists);
        assert!(s.dismount(fi, None));
        assert_eq!(s.combat.fighters[fi].right_actor, Some(actor));
        assert!(s.combat.fighters[fi].alternate_mode);
        assert!(Rc::ptr_eq(s.combat.fighters[fi].weapon.as_ref().unwrap(), &data));
        assert_eq!(s.combat.equipment_actor(fists).unwrap().placement, EquipmentPlacement::Holstered);
    }

    #[test]
    fn equipment_horse_destroyed_saved_actor_does_not_create_replacement() {
        let Some((mut s, hi)) = fixture() else { return };
        let fi = rider(&mut s, hi, "A");
        let actor = s.combat.fighters[fi].right_actor.unwrap();
        assert!(s.mount(fi, hi, (false, false, false), FISTS));
        let fists = s.combat.fighters[fi].right_actor;
        s.combat.destroy_equipment_actor(actor).unwrap();
        let live = s.combat.equipment_counts().live;
        assert!(s.dismount(fi, None));
        assert_eq!(s.combat.fighters[fi].right_actor, fists);
        assert_eq!(s.combat.equipment_counts().live, live);
        assert!(s.combat.equipment_actor(actor).is_none());
        assert!(s.combat.hits.iter().any(|e| e["kind"] == "equipment_error" && e["operation"] == "horse_restore"));
    }

    #[test]
    fn equipment_horse_respawn_and_earlier_removal_clear_or_shift_references() {
        let Some((mut s, hi)) = fixture() else { return };
        let first = rider(&mut s, hi, "A");
        let fi = rider(&mut s, hi, "B");
        let pawn = s.combat.fighters[fi].id;
        let actor = s.combat.fighters[fi].right_actor.unwrap();
        assert!(s.mount(fi, hi, (false, false, false), FISTS));
        s.horses[hi].last_used.push((fi, 0.0));
        s.horses[hi].hit_by.push((fi, 0.0));
        s.horses[hi].combat.recent_knockbacks.push((fi, 0.0));
        assert_eq!(first, 0);
        s.remove_fighter("A");
        assert_eq!(s.horses[hi].rider, Some(0));
        assert_eq!(s.horses[hi].m.horse.as_ref().unwrap().driver, Some(0));
        assert_eq!(s.horses[hi].last_used[0].0, 0);
        assert_eq!(s.horses[hi].hit_by[0].0, 0);
        assert_eq!(s.horses[hi].combat.recent_knockbacks[0].0, 0);
        assert_eq!(s.horses[hi].stored_right.unwrap().pawn_id, pawn);
        let (location, yaw) = s.horses[hi].m.horse_rider_seat();
        s.respawn(0, location, yaw);
        assert_eq!(s.horses[hi].rider, None);
        assert_eq!(s.horses[hi].stored_right, None);
        assert_eq!(s.horses[hi].m.horse.as_ref().unwrap().driver, None);
        assert!(s.horses[hi].last_used.is_empty());
        assert!(s.horses[hi].hit_by.is_empty());
        assert_ne!(s.combat.fighters[0].id, pawn);
        assert_eq!(s.combat.equipment_actor(actor).unwrap().placement, EquipmentPlacement::PawnCleanupPending);
        assert!(s.combat.equip_right_actor(0, actor).is_err());
    }
}

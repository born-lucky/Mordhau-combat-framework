//! Projectiles in the Sim (rust-combat r4): mh-character's projectile.rs flies and sweeps them
//! (UProjectileMovementComponent / AMordhauProjectile::SweepProjectile), the posed physics-asset bodies are the
//! targets, and every character hit goes through mordhau-core's ranged.rs (AMordhauProjectile::ProcessProjectileHit's
//! character branch: parry, ComputeRangedDamage, TakeDamage, flinch).
//!
//! Frame order: projectiles tick after the late tick (AMordhauProjectile::Tick rva=0x1600ff0 is an actor tick in
//! TG_PrePhysics; UNCONFIRMED relative to the characters' ticks, which the exe does not order), against this frame's
//! posed bodies. World gravity Z -980 (UE default WorldSettings; UNCONFIRMED per map).
//! After a character hit (ProcessProjectileHit_Implementation, decomp AMordhauProjectile.cpp 540-700): WillSticky(the
//! body's surface) -> AttachProjectile (stick) and terminate; else WillPassThrough -> the flight continues; else
//! terminate. A world hit: WillSticky -> stick, else terminate. Each character is hit at most once per projectile.
//! Body surfaces are 0 (SurfaceType_Default; the physical materials are not read: UNCONFIRMED). Shapes: capsules and
//! spheres as the swept capsule bodies; boxes as their bounding sphere (UNCONFIRMED: the exe sweeps the real box).
//! The ranged motions (URangedDrawMotion / URangedReleaseMotion / UReloadMotion) are not ported: hosts fire with
//! `Sim::fire_projectile` (URangedReleaseMotion::OnBegin rva=0x16620d0's spawn: origin = CameraLocation1P, rotation =
//! CameraRotation1P * AimRotationOffset).

use crate::physics::Shape;
use crate::sim::Sim;
use mh_character::projectile::{Projectile, ProjectileCfg, ProjectileTarget, TargetBody};
use mordhau_core::combat::ranged::{projectile_damage_from_defaults, ProjectileDamage, ProjectileOutcome};
use mordhau_core::ue::FVector;

pub const WORLD_GRAVITY_Z: f32 = -980.0;

/// One projectile in flight with its combat data
#[derive(Clone, Debug)]
pub struct SimProjectile {
    pub p: Projectile,
    pub shooter: Option<usize>,
    pub damage: ProjectileDamage,
    pub hit_fighters: Vec<usize>,
    /// outcomes of its character hits (fighter, outcome), for hosts and tests
    pub outcomes: Vec<(usize, ProjectileOutcome)>,
}

/// A projectile class from the paks: the flight config (ProjectileCfg::from_json_chain over the Blueprint chain) and
/// the damage data (the merged class defaults)
pub fn projectile_class(rd: &mh_pak::Reader, class: &str) -> Result<(ProjectileCfg, ProjectileDamage), String> {
    let chain = rd.chain(class);
    if chain.is_empty() {
        return Err(format!("projectile class {class} not in the paks"));
    }
    let jsons: Vec<String> = chain.iter().filter_map(|p| rd.read(p)).map(|ex| serde_json::Value::Array(ex).to_string()).collect();
    let refs: Vec<&str> = jsons.iter().map(|s| s.as_str()).collect();
    let cfg = ProjectileCfg::from_json_chain(&refs)?;
    let mut d = projectile_damage_from_defaults(&rd.defaults(class));
    // the Damage / HeadBonus / LegBonus arrays as mh-character's chain reader resolves them (rust-character r7: the
    // most derived class that sets each)
    if d.damage.is_empty() {
        d.damage = cfg.damage.clone();
    }
    if d.head_bonus.is_empty() {
        d.head_bonus = cfg.head_bonus.clone();
    }
    if d.leg_bonus.is_empty() {
        d.leg_bonus = cfg.leg_bonus.clone();
    }
    Ok((cfg, d))
}

impl Sim {
    /// fire a projectile now (FireProjectile_Internal's spawn + Fire_Implementation): origin UE cm, rotation degrees
    pub fn fire_projectile(&mut self, shooter: Option<usize>, cfg: ProjectileCfg, damage: ProjectileDamage, origin: FVector, pitch: f32, yaw: f32) -> usize {
        let p = Projectile::fire(cfg, origin, pitch, yaw, 0.0, self.combat.now as f32);
        self.projectiles.push(SimProjectile { p, shooter, damage, hit_fighters: Vec::new(), outcomes: Vec::new() });
        self.projectiles.len() - 1
    }

    /// give fighter `fi` its right-hand weapon's ranged data (AMordhauEquipment class defaults; rangedmotion.rs) and
    /// load its projectile class; false when the weapon cannot fire (bAllowFire false or no ProjectileClass)
    /// `weapon`: the ranged equipment class (the bows are not in the combat spec's melee weapon set, so the host
    /// names it; "" = the fighter's right-hand weapon)
    pub fn equip_ranged(&mut self, fi: usize, rd: &mh_pak::Reader, weapon: &str) -> bool {
        let w = if weapon.is_empty() { self.combat.fighters[fi].weapon_path.clone() } else { weapon.to_string() };
        let d = rd.defaults(&w);
        let mut r = mordhau_core::combat::rangedmotion::RangedEquip::from_defaults(&w, &d);
        // RangedDrawSway (+0x580): the UCurveVector's three FloatCurves (FloatCurves, FloatCurves[1], FloatCurves[2])
        if let Some(p) = d.get("RangedDrawSway").and_then(|v| v["ObjectPath"].as_str()) {
            let p = crate::physics::strip(p);
            let e = rd.read(&p).and_then(|ex| mh_pak::pkg::export_of(&ex, "CurveVector").cloned());
            if let Some(e) = e {
                let tail = |v: &serde_json::Value| v.as_str().map(|s| s.rsplit("::").next().unwrap_or(s).to_string()).unwrap_or_else(|| "RCCE_Constant".into());
                let fc = |k: &str| {
                    let c = &e["Properties"][k];
                    (serde_json::from_value::<Vec<mordhau_core::data::CurveKey>>(c["Keys"].clone()).unwrap_or_default(), tail(&c["PreInfinityExtrap"]), tail(&c["PostInfinityExtrap"]))
                };
                r.draw_sway = Some(std::rc::Rc::new([fc("FloatCurves"), fc("FloatCurves[1]"), fc("FloatCurves[2]")]));
            }
        }
        if !r.b_allow_fire || r.projectile_class.is_empty() {
            return false;
        }
        if !self.projectile_classes.contains_key(&r.projectile_class) {
            match projectile_class(rd, &r.projectile_class) {
                Ok(c) => {
                    self.projectile_classes.insert(r.projectile_class.clone(), c);
                }
                Err(_) => return false,
            }
        }
        self.combat.fighters[fi].ranged = Some(r);
        true
    }

    /// the release motions' fire requests -> projectiles (AMordhauEquipment::FireProjectile rva=0x153d950)
    pub(crate) fn spawn_fired(&mut self) {
        for f in self.combat.drain_fires() {
            let Some((cfg, dmg)) = self.projectile_classes.get(&f.projectile_class).cloned() else { continue };
            self.fire_projectile(Some(f.fighter), cfg, dmg, f.origin, f.rot.0, f.rot.1);
        }
    }

    /// every fighter's posed bodies as projectile targets (target id = fighter index)
    fn projectile_targets(&self) -> Vec<ProjectileTarget> {
        let posed = self.posed.borrow();
        let mut out = Vec::new();
        for (fi, f) in self.combat.fighters.iter().enumerate() {
            if f.dead {
                continue;
            }
            let Some(p) = posed.get(&f.name) else { continue };
            let mut bodies = Vec::new();
            for (k, s) in self.geo.shapes.iter().enumerate() {
                let Some(b) = self.geo.shape_bones[k] else { continue };
                let w = s.xf.then(&self.geo.bone_world(p, b));
                let (a, bb, r) = match s.shape {
                    Shape::Capsule { radius, half_len } => (w.apply(FVector::new(0.0, 0.0, -half_len)), w.apply(FVector::new(0.0, 0.0, half_len)), radius),
                    Shape::Sphere { radius } => (w.loc, w.loc, radius),
                    Shape::Box { half } => (w.loc, w.loc, half.length()),
                };
                bodies.push(TargetBody { bone: s.bone.clone(), a, b: bb, radius: r, surface: 0 });
            }
            out.push(ProjectileTarget { id: fi, bodies });
        }
        out
    }

    /// one frame of every live projectile (module docs)
    pub(crate) fn tick_projectiles(&mut self) {
        self.spawn_fired();
        if self.projectiles.is_empty() {
            return;
        }
        let targets = self.projectile_targets();
        let dt = self.dt;
        let mut list = std::mem::take(&mut self.projectiles);
        for sp in list.iter_mut() {
            if sp.p.terminated {
                continue;
            }
            let hits = sp.p.tick(self.collision.as_ref(), &targets, dt, WORLD_GRAVITY_Z);
            for h in hits {
                if sp.p.terminated {
                    break;
                }
                match &h.target {
                    Some((fi, bone)) => {
                        let fi = *fi;
                        if Some(fi) == sp.shooter || sp.hit_fighters.contains(&fi) {
                            continue;
                        }
                        sp.hit_fighters.push(fi);
                        let fwd = mh_character::uequat::quat_rotate(sp.p.quat, FVector::new(1.0, 0.0, 0.0));
                        let o = self.combat.projectile_hit(fi, sp.shooter, &sp.damage, bone, h.impact_point, sp.p.velocity, fwd);
                        sp.outcomes.push((fi, o.clone()));
                        if let ProjectileOutcome::Damaged { knockback, .. } = o {
                            if knockback != FVector::ZERO {
                                // AMordhauCharacter::Knockback (vcall +0x998): the movement side's launch, UNCONFIRMED
                                // as a plain velocity add
                                let m = &mut self.movers[fi];
                                m.velocity = m.velocity + knockback;
                            }
                        }
                        if matches!(o, ProjectileOutcome::Parried) {
                            sp.p.terminate();
                        } else if sp.p.will_sticky(h.surface) {
                            sp.p.stick(&h, FVector::ZERO);
                            sp.p.terminate();
                        } else if !sp.p.will_pass_through(h.surface) {
                            sp.p.terminate();
                        }
                    }
                    None => {
                        if sp.p.will_sticky(h.surface) {
                            sp.p.stick(&h, FVector::ZERO);
                        }
                        sp.p.terminate();
                    }
                }
            }
        }
        self.projectiles = list;
    }
}

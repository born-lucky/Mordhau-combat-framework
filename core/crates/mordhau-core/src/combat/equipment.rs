//! Equipment actors, distinct from shared cooked class records. Native weak references use object index/serial
//! (FWeakObjectPtr::Get rva=0x1b7d5a0); removing an actor from a hand or inventory does not invalidate it.
//! DropEquipment rva=0x14ba6d0 transfers that same actor. Actual destruction is an explicit host boundary;
//! the unrecovered death/deferred-cleanup callbacks are deliberately not replaced by a timer here.

use super::{World, tracer::Tracer};
use crate::data::{EquipmentDef, WeaponData};
use std::rc::Rc;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct EquipmentId {
    pub slot: u32,
    pub generation: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EquipmentPlacement { RightHand, LeftHand, Holstered, VirtualWeapon, Dropped, PawnCleanupPending }

#[derive(Clone, Debug)]
pub struct EquipmentInstance {
    pub path: String,
    /// Immutable native class from EquipmentDef/PDB ancestry. Empty means unknown, never known non-shield.
    pub native_class: String,
    pub weapon: Option<Rc<WeaponData>>,
    pub equip: Option<Rc<EquipmentDef>>,
    pub alternate_mode: bool,
    pub motion_bps: std::collections::HashMap<i64, String>,
    pub owner_pawn: Option<u32>,
    pub placement: EquipmentPlacement,
}

struct EquipmentSlot { generation: u64, actor: Option<EquipmentInstance> }

#[derive(Default)]
pub(crate) struct EquipmentArena { slots: Vec<EquipmentSlot>, free: Vec<u32> }

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EquipmentCounts { pub live: usize, pub dropped: usize, pub cleanup_pending: usize }

impl EquipmentArena {
    fn create(&mut self, actor: EquipmentInstance) -> Result<EquipmentId, String> {
        if let Some(slot) = self.free.pop() {
            let s = &mut self.slots[slot as usize];
            debug_assert!(s.actor.is_none());
            s.actor = Some(actor);
            Ok(EquipmentId { slot, generation: s.generation })
        } else {
            let slot = u32::try_from(self.slots.len()).map_err(|_| "equipment arena exhausted")?;
            self.slots.push(EquipmentSlot { generation: 1, actor: Some(actor) });
            Ok(EquipmentId { slot, generation: 1 })
        }
    }
    fn resolve(&self, id: EquipmentId) -> Option<&EquipmentInstance> {
        self.slots.get(id.slot as usize).filter(|s| s.generation == id.generation)?.actor.as_ref()
    }
    pub(crate) fn resolve_mut(&mut self, id: EquipmentId) -> Option<&mut EquipmentInstance> {
        self.slots.get_mut(id.slot as usize).filter(|s| s.generation == id.generation)?.actor.as_mut()
    }
    fn destroy(&mut self, id: EquipmentId) -> Result<(), String> {
        let s = self.slots.get_mut(id.slot as usize).filter(|s| s.generation == id.generation && s.actor.is_some())
            .ok_or_else(|| format!("stale equipment actor {id:?}"))?;
        s.actor = None;
        // A serial must never wrap back onto a stale weak handle. An exhausted slot is permanently retired.
        if let Some(next) = s.generation.checked_add(1) { s.generation = next; self.free.push(id.slot); }
        Ok(())
    }
}

impl World {
    pub fn equipment_actor(&self, id: EquipmentId) -> Option<&EquipmentInstance> { self.equipment.resolve(id) }

    /// Explicit rewrite authoring boundary: do not change captured attack/parry/blocked data.
    /// The caller retains a pending edit until every live pawn has completed its current motion.
    pub fn timing_edit_ready(&self) -> bool {
        self.fighters.iter().all(|f| f.health <= 0 || f.motion
            .and_then(|id| f.motions.get(id.0 as usize)).and_then(Option::as_ref)
            .is_none_or(|m| matches!(m.k, super::motion::MotionKind::Idle)))
    }

    pub fn install_timing_spec(&mut self, spec: Rc<crate::data::Spec>) -> Result<(), String> {
        if !self.timing_edit_ready() { return Err("combat timing reload is waiting for idle".into()); }
        for slot in &mut self.equipment.slots {
            let Some(actor) = &mut slot.actor else { continue };
            let Some(source) = spec.weapon(&actor.path) else { continue };
            let Some(weapon) = &mut actor.weapon else { continue };
            // Preserve instance-only flags such as bAllowDrop, and the exact actor/serial/blood identity.
            if actor.alternate_mode {
                crate::timing::copy_weapon_timings(Rc::make_mut(weapon), &source.switched());
            } else {
                crate::timing::copy_weapon_timings(Rc::make_mut(weapon), &source);
            }
        }
        self.spec = spec;
        for fi in 0..self.fighters.len() {
            self.project_equipment_hands(fi, false);
            if let Some(id) = self.fighters[fi].kick_actor {
                self.fighters[fi].kick_weapon = self.equipment_actor(id).and_then(|a| a.weapon.clone());
            }
        }
        Ok(())
    }

    pub fn equipment_counts(&self) -> EquipmentCounts {
        let mut out = EquipmentCounts::default();
        for a in self.equipment.slots.iter().filter_map(|s| s.actor.as_ref()) {
            out.live += 1;
            out.dropped += usize::from(a.placement == EquipmentPlacement::Dropped);
            out.cleanup_pending += usize::from(a.placement == EquipmentPlacement::PawnCleanupPending);
        }
        out
    }

    pub fn equipment_is_class(&self, id: EquipmentId, base: &str) -> bool {
        self.equipment_actor(id).is_some_and(|a| !a.native_class.is_empty() && self.spec.is_class_of(&a.native_class, base))
    }

    /// Explicit fresh creation; equal cooked paths/data Rcs do not imply equal actors.
    pub fn create_equipment(&mut self, pawn_id: u32, path: &str) -> Result<EquipmentId, String> {
        let fi = self.fighter_by_id(pawn_id).ok_or_else(|| format!("no pawn {pawn_id} owns new equipment"))?;
        let id = self.create_pawn_equipment(pawn_id, path)?;
        self.fighters[fi].inventory_actors.push(id);
        Ok(id)
    }
    pub(crate) fn create_pawn_equipment(&mut self, pawn_id: u32, path: &str) -> Result<EquipmentId, String> {
        let s = self.spec.weapons.get(path).ok_or_else(|| format!("no equipment records for {path}"))?;
        let native = s.equip.as_ref().map(|e| e.native.as_str()).filter(|n| !n.is_empty())
            .unwrap_or(&s.weapon.native_class).to_string();
        let weapon = if s.equip.as_ref().is_none_or(|e| e.is_weapon) { Some(s.weapon.clone()) } else { None };
        self.equipment.create(EquipmentInstance { path: path.to_string(), native_class: native,
            weapon, equip: s.equip.clone(), alternate_mode: false, motion_bps: s.motions.clone(), owner_pawn: Some(pawn_id),
            placement: EquipmentPlacement::Holstered })
    }

    fn validate_transfer(&self, fi: usize, id: EquipmentId, right: bool) -> Result<(), String> {
        let f = self.fighters.get(fi).ok_or_else(|| format!("no fighter {fi}"))?;
        let a = self.equipment_actor(id).ok_or_else(|| format!("stale equipment actor {id:?}"))?;
        if a.placement == EquipmentPlacement::PawnCleanupPending {
            return Err(format!("equipment actor {id:?} awaits pawn cleanup"));
        }
        if a.owner_pawn.is_some_and(|owner| owner != f.id) {
            return Err(format!("equipment actor {id:?} belongs to another pawn"));
        }
        if a.owner_pawn.is_none() && a.placement != EquipmentPlacement::Dropped {
            return Err(format!("equipment actor {id:?} has no transferable placement"));
        }
        if right && a.weapon.is_none() { return Err(format!("equipment actor {id:?} is not a right-hand weapon")); }
        Ok(())
    }

    /// Transfer an existing actor; preserve its class, generation and current mode/data.
    pub fn equip_right_actor(&mut self, fi: usize, id: EquipmentId) -> Result<(), String> {
        self.validate_transfer(fi, id, true)?;
        self.equip_actor(fi, id, true)
    }
    pub fn equip_left_actor(&mut self, fi: usize, id: EquipmentId) -> Result<(), String> {
        self.validate_transfer(fi, id, false)?;
        self.equip_actor(fi, id, false)
    }
    fn equip_actor(&mut self, fi: usize, id: EquipmentId, right: bool) -> Result<(), String> {
        let old = if right { self.fighters[fi].right_actor } else { self.fighters[fi].left_actor };
        if let Some(old) = old.filter(|old| *old != id) { self.holster_actor(fi, old)?; }
        let pawn = self.fighters[fi].id;
        let f = &mut self.fighters[fi];
        if f.right_actor == Some(id) { f.right_actor = None; }
        if f.left_actor == Some(id) { f.left_actor = None; }
        if right { f.right_actor = Some(id); } else { f.left_actor = Some(id); }
        if !f.inventory_actors.contains(&id) { f.inventory_actors.push(id); }
        let a = self.equipment.resolve_mut(id).expect("validated actor");
        a.owner_pawn = Some(pawn);
        a.placement = if right { EquipmentPlacement::RightHand } else { EquipmentPlacement::LeftHand };
        self.project_equipment_hands(fi, right);
        Ok(())
    }

    pub fn create_and_equip_right(&mut self, fi: usize, path: &str) -> Result<EquipmentId, String> {
        let pawn = self.fighters.get(fi).ok_or_else(|| format!("no fighter {fi}"))?.id;
        // Validate the class-data operation before allocating, so an invalid request leaves no orphan actor.
        let s = self.spec.weapons.get(path).ok_or_else(|| format!("no equipment records for {path}"))?;
        if s.equip.as_ref().is_some_and(|e| !e.is_weapon) { return Err(format!("{path} is not a right-hand weapon")); }
        let id = self.create_equipment(pawn, path)?;
        self.equip_right_actor(fi, id)?;
        Ok(id)
    }

    pub fn holster_actor(&mut self, fi: usize, id: EquipmentId) -> Result<(), String> {
        let pawn = self.fighters.get(fi).ok_or_else(|| format!("no fighter {fi}"))?.id;
        let a = self.equipment.resolve_mut(id).ok_or_else(|| format!("stale equipment actor {id:?}"))?;
        if a.owner_pawn != Some(pawn) || a.placement == EquipmentPlacement::PawnCleanupPending {
            return Err(format!("equipment actor {id:?} is not owned by pawn {pawn}"));
        }
        a.placement = EquipmentPlacement::Holstered;
        let f = &mut self.fighters[fi];
        if f.right_actor == Some(id) { f.right_actor = None; }
        if f.left_actor == Some(id) { f.left_actor = None; }
        if !f.inventory_actors.contains(&id) { f.inventory_actors.push(id); }
        self.project_equipment_hands(fi, false);
        Ok(())
    }

    /// Known drop transition, not a gameplay request/can-drop policy. OnDropped mode reversion/physics are host gaps.
    pub fn drop_actor(&mut self, id: EquipmentId) -> Result<(), String> {
        let a = self.equipment.resolve_mut(id).ok_or_else(|| format!("stale equipment actor {id:?}"))?;
        a.owner_pawn = None;
        a.placement = EquipmentPlacement::Dropped;
        self.clear_actor_membership(id, false);
        Ok(())
    }

    pub fn destroy_equipment_actor(&mut self, id: EquipmentId) -> Result<(), String> {
        self.equipment.destroy(id)?;
        self.clear_actor_membership(id, true);
        Ok(())
    }
    fn clear_actor_membership(&mut self, id: EquipmentId, destroyed: bool) {
        for fi in 0..self.fighters.len() {
            let f = &mut self.fighters[fi];
            let held = f.right_actor == Some(id) || f.left_actor == Some(id);
            if f.right_actor == Some(id) { f.right_actor = None; }
            if f.left_actor == Some(id) { f.left_actor = None; }
            f.inventory_actors.retain(|actor| *actor != id);
            if f.fists_actor == Some(id) { f.fists_actor = None; }
            if destroyed && f.kick_actor == Some(id) { f.kick_actor = None; f.kick_weapon = None; }
            if held { self.project_equipment_hands(fi, false); }
        }
    }

    /// Materialize the designated virtual fists once, then transfer that actor. Never deduplicate arbitrary paths.
    pub fn designated_fists_actor(&mut self, fi: usize, path: &str) -> Result<EquipmentId, String> {
        let f = self.fighters.get(fi).ok_or_else(|| format!("no fighter {fi}"))?;
        if let Some(id) = f.fists_actor {
            let a = self.equipment_actor(id).ok_or_else(|| format!("stale designated fists {id:?}"))?;
            if a.path != path { return Err("different designated fists class requires explicit replacement".into()); }
            return Ok(id);
        }
        let pawn = f.id;
        let id = self.create_equipment(pawn, path)?;
        self.fighters[fi].fists_actor = Some(id);
        Ok(id)
    }

    pub(crate) fn release_pawn_equipment(&mut self, fi: usize) {
        let pawn = self.fighters[fi].id;
        for s in &mut self.equipment.slots {
            if let Some(a) = &mut s.actor {
                if a.owner_pawn == Some(pawn) {
                    a.owner_pawn = None;
                    a.placement = EquipmentPlacement::PawnCleanupPending;
                }
            }
        }
        let f = &mut self.fighters[fi];
        f.right_actor = None; f.left_actor = None; f.fists_actor = None; f.kick_actor = None;
        f.kick_weapon = None; f.inventory_actors.clear();
        self.project_equipment_hands(fi, false);
    }

    /// The only hand-data projection. A null right hand keeps the legacy respawn path/profile/tracer cache;
    /// weapon/equipment Options and actor tokens are the authoritative held-state views.
    pub(crate) fn project_equipment_hands(&mut self, fi: usize, reset_tracer: bool) {
        let r = self.fighters[fi].right_actor.and_then(|id| self.equipment_actor(id)).cloned();
        let l = self.fighters[fi].left_actor.and_then(|id| self.equipment_actor(id)).cloned();
        let f = &mut self.fighters[fi];
        f.weapon = r.as_ref().and_then(|a| a.weapon.clone());
        f.weapon_equip = r.as_ref().and_then(|a| a.equip.clone());
        if let Some(a) = r {
            f.weapon_path = a.path.clone(); f.alternate_mode = a.alternate_mode;
            f.motion_bps = a.motion_bps.clone();
            if reset_tracer { f.tracer = Tracer::new(&self.spec.weapon_setup(&a.path).tracer, &self.spec.constants); }
        }
        f.left_hand_path = l.as_ref().map(|a| a.path.clone()).unwrap_or_default();
        f.left_hand = l.as_ref().and_then(|a| a.equip.clone());
        f.left_weapon = l.as_ref().and_then(|a| a.weapon.clone());
        self.fighters[fi].parry_bp = self.parry_motion_bp(fi);
    }

    pub(crate) fn set_weapon_no_drop(&mut self, fi: usize) {
        let id = self.fighters[fi].right_actor.expect("weapon_no_drop without right actor");
        let a = self.equipment.resolve_mut(id).expect("right actor must be live");
        let mut weapon = (**a.weapon.as_ref().expect("right actor without weapon")).clone();
        weapon.b_allow_drop = false; a.weapon = Some(Rc::new(weapon));
        self.project_equipment_hands(fi, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::data::{RecordsJsonExe, Spec, SpecSource};
    use crate::combat::{enums::mv, world::{Call, Input}};

    const LS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";
    const FISTS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/BP_FistsWeapon";
    fn original_world() -> Option<World> {
        let p = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/spec.json");
        let required = std::env::var("MORDHAU_GOLDEN_REQUIRED").is_ok_and(|v| v == "1");
        let txt = match std::fs::read_to_string(&p) {
            Ok(txt) => txt,
            Err(error) => { assert!(!required, "required original equipment records {}: {error}", p.display()); return None; }
        };
        Some(World::new(Rc::new(RecordsJsonExe(&txt).load_spec().expect("original records")), 1.0/120.0))
    }

    #[test]
    fn equipment_same_class_is_not_same_actor_and_stale_generation_cannot_resolve() {
        let Some(mut w) = original_world() else { return };
        let fi = w.add_fighter("A", LS, LS);
        let (r, l) = (w.fighters[fi].right_actor.unwrap(), w.fighters[fi].left_actor.unwrap());
        assert_ne!(r, l);
        assert!(Rc::ptr_eq(w.equipment_actor(r).unwrap().weapon.as_ref().unwrap(), w.equipment_actor(l).unwrap().weapon.as_ref().unwrap()));
        assert!(w.equipment_is_class(r, "AMordhauWeapon"));
        w.destroy_equipment_actor(r).unwrap();
        assert!(w.equipment_actor(r).is_none());
        assert_eq!(w.fighters[fi].right_actor, None);
        assert_eq!(w.fighters[fi].left_actor, Some(l));
        assert!(w.fighters[fi].left_weapon.is_some());
        let replacement = w.create_and_equip_right(fi, LS).unwrap();
        assert_eq!(replacement.slot, r.slot);
        assert_ne!(replacement.generation, r.generation);
        assert!(w.equip_right_actor(fi, r).is_err());
        assert_eq!(w.fighters[fi].right_actor, Some(replacement));
        assert!(w.equipment_actor(l).is_some());
    }

    #[test]
    fn equipment_holster_transfer_and_mode_keep_actor_data() {
        let Some(mut w) = original_world() else { return };
        let fi = w.add_fighter("A", LS, "");
        let id = w.fighters[fi].right_actor.unwrap();
        let class = w.equipment_actor(id).unwrap().native_class.clone();
        w.switch_mode_and_reattach(fi);
        assert_eq!(w.fighters[fi].right_actor, Some(id));
        assert!(w.equipment_actor(id).unwrap().alternate_mode);
        let mode_weapon = w.equipment_actor(id).unwrap().weapon.clone().unwrap();
        let mode_equip = w.equipment_actor(id).unwrap().equip.clone().unwrap();
        w.holster_actor(fi, id).unwrap();
        assert!(w.equipment_actor(id).is_some());
        assert!(w.fighters[fi].weapon.is_none());
        let fists = w.designated_fists_actor(fi, FISTS).unwrap();
        assert_eq!(w.designated_fists_actor(fi, FISTS).unwrap(), fists);
        w.equip_right_actor(fi, fists).unwrap();
        w.equip_right_actor(fi, id).unwrap();
        assert_eq!(w.equipment_actor(id).unwrap().native_class, class);
        assert!(w.fighters[fi].alternate_mode);
        assert!(Rc::ptr_eq(w.fighters[fi].weapon.as_ref().unwrap(), &mode_weapon));
        assert!(Rc::ptr_eq(w.fighters[fi].weapon_equip.as_ref().unwrap(), &mode_equip));
        assert_eq!(w.equipment_actor(fists).unwrap().placement, EquipmentPlacement::Holstered);
        assert_eq!(w.fighters[fi].motion_bps, w.equipment_actor(id).unwrap().motion_bps);
    }

    #[test]
    fn equipment_drop_retains_actor_and_cleanup_is_membership_only() {
        let Some(mut w) = original_world() else { return };
        let a = w.add_fighter("A", LS, "");
        let b = w.add_fighter("B", LS, "");
        let dropped = w.fighters[a].right_actor.unwrap();
        assert!(w.equip_right_actor(b, dropped).is_err(), "other pawn cannot steal an owned actor");
        w.drop_actor(dropped).unwrap();
        assert!(w.equipment_actor(dropped).is_some());
        assert_eq!(w.equipment_actor(dropped).unwrap().owner_pawn, None);
        assert!(!w.fighters[a].inventory_actors.contains(&dropped));
        w.respawn_fighter(a);
        let fresh = w.fighters[a].right_actor.unwrap();
        assert_ne!(fresh, dropped);
        assert_eq!(w.equipment_actor(dropped).unwrap().placement, EquipmentPlacement::Dropped);
        let retired = w.fighters[b].right_actor.unwrap();
        w.remove_fighter("B");
        assert_eq!(w.equipment_actor(retired).unwrap().placement, EquipmentPlacement::PawnCleanupPending);
        assert!(w.equipment_actor(retired).is_some(), "remove does not invent native destruction");
        assert!(w.equip_right_actor(a, retired).is_err());
        w.equip_right_actor(a, dropped).unwrap();
        assert_eq!(w.fighters[a].right_actor, Some(dropped));
        assert_eq!(w.equipment_actor(dropped).unwrap().owner_pawn, Some(w.fighters[a].id));
        assert!(w.equipment_counts().cleanup_pending > 0);
    }

    #[test]
    fn equipment_removal_does_not_rebind_other_pawn_actor_to_shifted_index() {
        let Some(mut w) = original_world() else { return };
        w.add_fighter("A", LS, "");
        let b = w.add_fighter("B", LS, "");
        let pawn = w.fighters[b].id;
        let actor = w.fighters[b].right_actor.unwrap();
        w.remove_fighter("A");
        assert_eq!(w.fighters[0].id, pawn);
        assert_eq!(w.equipment_actor(actor).unwrap().owner_pawn, Some(pawn));
        w.holster_actor(0, actor).unwrap();
        w.equip_right_actor(0, actor).unwrap();
        assert_eq!(w.fighters[0].right_actor, Some(actor));
    }

    #[test]
    fn equipment_attack_disarm_uses_exact_origin_and_stale_origin_does_not_fall_back() {
        let Some(mut w) = original_world() else { return };
        let fi = w.add_fighter("A", LS, "");
        w.request_attack(fi, mv::STAB, 0.0);
        let attack = w.cur(fi).unwrap();
        let origin = w.m(fi, attack).attack().unwrap().weapon_actor.unwrap();
        let other = w.create_and_equip_right(fi, LS).unwrap();
        w.equip_left_actor(fi, origin).unwrap();
        w.drop_for_disarm(fi, Some(attack));
        assert_eq!(w.fighters[fi].right_actor, Some(other));
        assert_eq!(w.fighters[fi].left_actor, None);
        assert_eq!(w.equipment_actor(origin).unwrap().placement, EquipmentPlacement::Dropped);
        w.destroy_equipment_actor(origin).unwrap();
        let events = w.hits.len();
        w.drop_for_disarm(fi, Some(attack));
        assert_eq!(w.fighters[fi].right_actor, Some(other));
        assert_eq!(w.hits.len(), events, "stale capture does not drop same-class replacement");
    }

    #[test]
    fn equipment_kick_origin_is_undroppable_without_current_hand_fallback() {
        let Some(mut w) = original_world() else { return };
        let fi = w.add_fighter("A", LS, "");
        w.request_attack(fi, mv::KICK, 0.0);
        let attack = w.cur(fi).unwrap();
        assert_eq!(w.m(fi, attack).attack().unwrap().weapon_actor, w.fighters[fi].kick_actor);
        let right = w.fighters[fi].right_actor;
        let events = w.hits.len();
        w.drop_for_disarm(fi, Some(attack));
        assert_eq!(w.fighters[fi].right_actor, right);
        assert_eq!(w.hits.len(), events);
    }

    #[test]
    fn equipment_no_drop_hook_updates_actor_and_held_view_together() {
        let Some(mut w) = original_world() else { return };
        let fi = w.add_fighter("A", LS, "");
        let id = w.fighters[fi].right_actor.unwrap();
        w.at(w.dt, Input::Call(Call::WeaponNoDrop { who: "A".into() }));
        w.step();
        assert!(!w.equipment_actor(id).unwrap().weapon.as_ref().unwrap().b_allow_drop);
        assert!(Rc::ptr_eq(w.fighters[fi].weapon.as_ref().unwrap(), w.equipment_actor(id).unwrap().weapon.as_ref().unwrap()));
        w.drop_for_disarm(fi, None);
        assert_eq!(w.fighters[fi].right_actor, Some(id));
    }

    #[test]
    fn equipment_unknown_class_is_not_a_known_weapon_or_non_shield() {
        let mut spec = Spec::default();
        spec.weapons.insert("synthetic_unknown".into(), crate::data::WeaponSetup {
            weapon: Rc::new(WeaponData::default()), equip: None, motions: Default::default(), tracer: Default::default()
        });
        let mut w = World::new(Rc::new(spec), 1.0/120.0);
        let id = w.create_pawn_equipment(1, "synthetic_unknown").unwrap();
        assert!(!w.equipment_is_class(id, "AMordhauWeapon"));
        assert!(!w.equipment_is_class(id, "AMordhauShield"));
        assert_eq!(w.equipment_actor(id).unwrap().native_class, "");
    }

    #[test]
    fn equipment_generation_overflow_retires_slot() {
        let actor = EquipmentInstance { path: "synthetic".into(), native_class: "AMordhauWeapon".into(),
            weapon: None, equip: None, alternate_mode: false, motion_bps: Default::default(), owner_pawn: None, placement: EquipmentPlacement::Dropped };
        let mut arena = EquipmentArena::default();
        let id = arena.create(actor.clone()).unwrap();
        arena.slots[id.slot as usize].generation = u64::MAX;
        let last = EquipmentId { generation: u64::MAX, ..id };
        arena.destroy(last).unwrap();
        let new = arena.create(actor).unwrap();
        assert_ne!(new.slot, last.slot);
        assert!(arena.resolve(last).is_none());
    }
}

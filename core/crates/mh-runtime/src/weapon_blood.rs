//! AMordhauWeapon::IncreaseBloodLevel (RVA0x162b850), persistent per actor/MID.
//! Native fresh UObject storage is zeroed; normal Greatsword's entire CDO chain
//! leaves both channel flags false. Source proofs: state/proofs/weapon-blood-plan-20261008.md.
use bevy::prelude::*;
use mordhau_core::combat::EquipmentId;
use std::collections::HashMap;

#[derive(Default, Clone, serde::Serialize)]
pub struct BloodState {
    pub levels: [f32; 2],
    pub stages: [f32; 2],
}

impl BloodState {
    /// Half stages accumulate, but native MID writes occur only at exact 1,2,3.
    fn increase(&mut self, channel: usize, show_blood: bool, surface: u64) {
        if !show_blood || surface != 1 || self.levels[channel] >= 3.0 { return; }
        self.levels[channel] = (self.levels[channel] + 0.5).min(3.0);
        if matches!(self.levels[channel], 1.0 | 2.0 | 3.0) {
            self.stages[channel] = self.levels[channel];
        }
    }
}

#[derive(Clone, serde::Serialize)]
pub struct BloodObservation {
    pub slot: u32,
    pub generation: u64,
    pub class: String,
    pub alternate: bool,
    pub channel: usize,
    pub surface: u64,
    pub show_blood: bool,
    pub levels: [f32; 2],
    pub stages: [f32; 2],
}

#[derive(Clone, serde::Serialize)]
pub struct BloodMaterialObservation {
    pub fighter: u32,
    pub left: bool,
    pub slot: u32,
    pub generation: u64,
    pub material: String,
    pub enabled: bool,
    pub stages: [f32; 2],
    pub textures: [String; 3],
}

#[derive(Resource, Default)]
pub struct WeaponBlood {
    pub actors: HashMap<EquipmentId, BloodState>,
    pub hits: u64,
    pub last_hit: Option<BloodObservation>,
    pub materials: Vec<BloodMaterialObservation>,
}

impl WeaponBlood {
    pub fn on_event(&mut self, ev: &serde_json::Value, rd: &mh_pak::Reader, show_blood: bool) {
        if ev["kind"].as_str() != Some("hit") || ev["ranged"].as_bool() == Some(true) { return; }
        let (Some(slot), Some(generation), Some(class), Some(alternate), Some(surface)) = (
            ev["weapon_actor"]["slot"].as_u64(), ev["weapon_actor"]["generation"].as_u64(),
            ev["weapon_class"].as_str(), ev["weapon_alternate"].as_bool(), ev["surface"].as_u64(),
        ) else { return; };
        let Ok(slot) = u32::try_from(slot) else { return; };
        let id = EquipmentId { slot, generation };
        let d = rd.defaults(class);
        // SwitchMode swaps these flags, not levels. Use the event's captured mode,
        // never the current hand/mode when the event is drained.
        let field = if alternate { "bSecondRegularAttacksUseBlood2" } else { "bRegularAttacksUseBlood2" };
        let channel = usize::from(d.get(field).and_then(|v| v.as_bool()).unwrap_or(false));
        let state = self.actors.entry(id).or_default();
        state.increase(channel, show_blood, surface);
        self.hits += 1;
        self.last_hit = Some(BloodObservation { slot, generation, class: class.into(), alternate, channel,
            surface, show_blood, levels: state.levels, stages: state.stages });
    }
}

/// Write the drawn actor's own MID after held meshes have been created/replaced.
/// No clearing on holster, transfer, perspective change, or renderer reconstruction.
pub fn update(
    sim: NonSend<crate::sim::Sim>,
    parts: Query<(&crate::weapon::SmearPart, &MeshMaterial3d<crate::uetint::UeTintMaterial>)>,
    mut mats: ResMut<Assets<crate::uetint::UeTintMaterial>>,
    mut blood: ResMut<WeaponBlood>,
) {
    let Some(world) = sim.0.combat() else { return; };
    blood.actors.retain(|id, _| world.equipment_actor(*id).is_some());
    blood.materials.clear();
    let idx = |name| mh_assets::shader::uniform_index(name).expect("weapon blood shader contract");
    for (part, handle) in &parts {
        let Some(fi) = sim.0.fighter_index(part.fighter) else { continue; };
        let Some(fighter) = world.fighters.get(fi) else { continue; };
        let Some(id) = (if part.left { fighter.left_actor } else { fighter.right_actor }) else { continue; };
        let Some(mut mat) = mats.get_mut(&handle.0) else { continue; };
        let stages = blood.actors.get(&id).map_or([0.0; 2], |s| s.stages);
        mat.u.v[idx("blood_stage1")].x = stages[0];
        mat.u.v[idx("blood_stage2")].x = stages[1];
        blood.materials.push(BloodMaterialObservation {
            fighter: part.fighter, left: part.left, slot: id.slot, generation: id.generation,
            material: format!("{:?}", handle.0.id()), enabled: mat.u.v[idx("weapon_blood_on")].x > 0.5,
            stages: [mat.u.v[idx("blood_stage1")].x, mat.u.v[idx("blood_stage2")].x],
            textures: [format!("{:?}", mat.t4.id()), format!("{:?}", mat.t5.id()), format!("{:?}", mat.t6.id())],
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn weapon_blood_native_six_flesh_hits_publish_only_integer_stages() {
        let mut s = BloodState::default();
        for (level, stage) in [(0.5,0.0),(1.0,1.0),(1.5,1.0),(2.0,2.0),(2.5,2.0),(3.0,3.0),(3.0,3.0)] {
            s.increase(0, true, 1);
            assert_eq!(s.levels, [level, 0.0]);
            assert_eq!(s.stages, [stage, 0.0]);
        }
    }
    #[test]
    fn weapon_blood_gore_surface_channel_and_actor_generation_are_independent() {
        let mut s = BloodState::default();
        s.increase(0, false, 1); s.increase(0, true, 0); s.increase(0, true, 2);
        assert_eq!(s.levels, [0.0;2]);
        s.increase(1, true, 1); s.increase(1, true, 1);
        assert_eq!(s.stages, [0.0,1.0]);
        let first = EquipmentId { slot: 4, generation: 1 };
        let replacement = EquipmentId { slot: 4, generation: 2 };
        let mut actors = HashMap::new(); actors.insert(first, s);
        assert_eq!(actors.entry(replacement).or_insert_with(BloodState::default).stages, [0.0;2]);
        assert_eq!(actors[&first].stages, [0.0,1.0]);
    }
}

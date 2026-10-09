//! KeepWorldPosition component attachment for world-space Cascade emitters.
//! Native SpawnEquipmentParticlesAttached0x156d920 -> SpawnEmitterAttached with
//! EAttachLocation1. Existing particles remain in world space; subsequent births
//! use the attached component's refreshed origin/full orientation. Local-space
//! attachment is explicitly unsupported until its native module semantics are ported.
use bevy::math::Affine3A;
use bevy::prelude::*;
use crate::sim::SystemSim;

#[derive(Clone, Copy, Debug)]
pub struct ComponentAttachment {
    pub parent: Entity,
    relative_origin: Vec3,
    relative_basis: [Vec3;3],
}
fn ue_direction(v:[f32;3])->Vec3 { Vec3::new(v[0],v[2],v[1]) }
fn from_bevy_direction(v:Vec3)->[f32;3] { [v.x,v.z,v.y] }

impl ComponentAttachment {
    pub fn keep_world(parent:Entity,birth_parent:Affine3A,sim:&SystemSim)->Self {
        let inv=birth_parent.inverse();
        let origin=ue_direction(sim.origin)*0.01;
        Self {parent,relative_origin:inv.transform_point3(origin),
            relative_basis:sim.basis.map(|v|inv.transform_vector3(ue_direction(v)))}
    }
    pub fn refresh(&self,parent:Affine3A,sim:&mut SystemSim) {
        let origin=parent.transform_point3(self.relative_origin)*100.0;
        sim.origin=from_bevy_direction(origin);
        sim.basis=self.relative_basis.map(|v|from_bevy_direction(parent.transform_vector3(v).normalize_or_zero()));
        // Do not transform existing Particle.pos/vel. They were born in world space.
    }
}

pub fn world_space_only(ps:&mh_assets::particles::ParticleSystem)->bool {
    ps.emitters.iter().all(|e|e.lods.first().and_then(|l|l.required.as_ref())
        .and_then(|r|r.props.get("bUseLocalSpace")).and_then(serde_json::Value::as_bool)!=Some(true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use mh_assets::particles::{Emitter,Lod,Module,ParticleSystem};
    use serde_json::json;
    fn system(local:bool)->ParticleSystem {
        let module=|class:&str,props|Module {class:class.into(),name:class.into(),props,dists:Default::default()};
        ParticleSystem {package:"synthetic attachment geometry".into(),props:json!({}),lod_distances:vec![],
            emitters:vec![Emitter {name:"synthetic".into(),class:"ParticleSpriteEmitter".into(),props:json!({}),lods:vec![Lod {
                required:Some(module("ParticleModuleRequired",json!({"EmitterDuration":2.0,"EmitterLoops":1,"bUseLocalSpace":local}))),
                spawn:Some(module("ParticleModuleSpawn",json!({"BurstList":[{"Count":1,"Time":0.0},{"Count":1,"Time":0.5}]}))),
                type_data:None,modules:vec![]}]}]}
    }
    #[test]
    fn attached_origin_follows_parent_but_born_world_particles_stay_put() {
        let mut sim=SystemSim::new(&system(false),[100.0,200.0,300.0],[1.0,0.0,0.0],1);
        let parent=World::new().spawn_empty().id();
        let attachment=ComponentAttachment::keep_world(parent,Affine3A::IDENTITY,&sim);
        sim.step(0.02);assert_eq!(sim.alive(),1);
        let first=sim.emitters[0].particles[0].pos;
        attachment.refresh(Affine3A::from_translation(Vec3::new(2.0,3.0,4.0)),&mut sim);
        assert_eq!(sim.origin,[300.0,600.0,600.0]);
        sim.step(0.6);assert_eq!(sim.alive(),2);
        assert_eq!(sim.emitters[0].particles[0].pos,first);
        assert_eq!(sim.emitters[0].particles[1].pos,[300.0,600.0,600.0]);
    }
    #[test]
    fn attachment_keeps_world_pose_at_birth_then_preserves_parent_roll() {
        let mut sim=SystemSim::new(&system(false),[100.0,0.0,0.0],[1.0,0.0,0.0],1);
        let parent=World::new().spawn_empty().id();
        let birth=Affine3A::from_rotation_translation(Quat::from_rotation_y(0.5),Vec3::new(2.0,3.0,4.0));
        let attachment=ComponentAttachment::keep_world(parent,birth,&sim);let origin=sim.origin;let basis=sim.basis;
        attachment.refresh(birth,&mut sim);
        assert!((Vec3::from_array(sim.origin)-Vec3::from_array(origin)).length()<0.0001);
        for (a,b)in sim.basis.iter().zip(basis){assert!((Vec3::from_array(*a)-Vec3::from_array(b)).length()<0.00001);}
        // A90 degree world rotation around X changes BOTH up/right, even though forward is unchanged.
        let moved=Affine3A::from_quat(Quat::from_rotation_x(std::f32::consts::FRAC_PI_2))*birth;
        attachment.refresh(moved,&mut sim);
        assert!((Vec3::from_array(sim.basis[0])-Vec3::X).length()<0.00001);
        assert!((Vec3::from_array(sim.basis[2])-Vec3::new(0.0,1.0,0.0)).length()<0.00001);
    }
    #[test]
    fn local_space_attachment_is_not_silently_simulated_as_world_space() {
        assert!(world_space_only(&system(false)));assert!(!world_space_only(&system(true)));
    }
}

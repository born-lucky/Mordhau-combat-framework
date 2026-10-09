//! Original PhysX corpse bodies/joints and death animation → BeginRagdoll transition.
//! Reviewed evidence: state/proofs/corpse_backend.md, ragdoll.md, docs/COMBAT_CONTINUATION.md.
//! Remaining parity gaps (not solver substitutions): death-graph procedural controls, offscreen rendering gate,
//! dismembered bodies, projectiles/vehicles and corpse-budget cleanup.

use crate::{Sim, pose::Skeleton};
use mh_assets::physics::{PhysicsAsset, Constraint, LimitDynamics};
use mh_pak::Reader;
use mh_physics::{Scene, Body, Shape, Joint, Transform};
use mordhau_core::ue::{FTransform, FQuat, FVector};
use std::path::Path;

const ASSET: &str = "Mordhau/Content/UMA/UMA/Master/UMA_Master_RagdollPhysicsAsset_Proper";
const RAW: &str = "Mordhau/Content/Mordhau/Animations/RawClips/Misc/";
const RETARGET: &str = "Mordhau/Content/Mordhau/Animations/Retargets/";

pub struct Corpse {
    pub fighter: usize,
    pub sequence: String,
    pub sequence_time: f64,
    pub started: f64,
    pub blend_started: Option<f64>,
    pub bodies: Vec<(usize, u32)>,
    pub weight: f32,
    pub initial_world: Vec<FTransform>,
    pub killing_impulse: Option<(String,FVector,FVector)>,
    pub impulse_applied: bool,
}
pub struct Ragdolls {
    pub scene: Scene,
    asset: PhysicsAsset,
    pub force_multipliers: std::collections::HashMap<String,f32>,
    pub force_ragdoll: std::collections::HashSet<String>,
    pub corpses: Vec<Corpse>,
    pub errors: Vec<String>,
    pub world_shapes: usize,
    pub skipped_world_shapes: usize,
}

fn clamp_size(v: FVector,max: f32) -> FVector { let len=v.length();if len>max {v.scale((max/len) as f64)} else {v} }
fn native(t: FTransform) -> Transform { Transform { position: [t.loc.x,t.loc.y,t.loc.z], rotation: [t.rot.x,t.rot.y,t.rot.z,t.rot.w] } }
fn engine(t: Transform) -> FTransform { FTransform::new(FQuat::from_array(t.rotation), FVector::new(t.position[0],t.position[1],t.position[2])) }
fn local(pos: [f64;3], rot: [f64;3]) -> Transform {
    native(FTransform::new(FQuat::from_rotator(rot[0] as f32,rot[1] as f32,rot[2] as f32), FVector::new(pos[0] as f32,pos[1] as f32,pos[2] as f32)))
}
fn offset(radius: f32) -> f32 { (radius * 0.01).clamp(0.0001,1.0) }
fn motion(s: &str) -> u32 { if s.ends_with("Locked") {0} else if s.ends_with("Limited") {1} else {2} }
fn rad(deg: f64) -> f32 { (deg * std::f64::consts::PI / 180.0) as f32 }

/// UE joint primary/secondary axes form the local frame's first two matrix columns (FConstraintInstance).
fn frame(p: [f64;3], x: [f64;3], y: [f64;3]) -> Transform {
    let z=[x[1]*y[2]-x[2]*y[1],x[2]*y[0]-x[0]*y[2],x[0]*y[1]-x[1]*y[0]];
    let m=[[x[0],y[0],z[0]],[x[1],y[1],z[1]],[x[2],y[2],z[2]]];
    let t=m[0][0]+m[1][1]+m[2][2];
    let q=if t>0. {
        let s=(1.+t).sqrt()*2.; [(m[2][1]-m[1][2])/s,(m[0][2]-m[2][0])/s,(m[1][0]-m[0][1])/s,s/4.]
    } else {
        let i=if m[0][0]>m[1][1] && m[0][0]>m[2][2] {0} else if m[1][1]>m[2][2] {1} else {2};
        let j=(i+1)%3; let k=(j+1)%3; let s=(1.+m[i][i]-m[j][j]-m[k][k]).sqrt()*2.;
        let mut q=[0.;4];q[i]=s/4.;q[j]=(m[j][i]+m[i][j])/s;q[k]=(m[k][i]+m[i][k])/s;q[3]=(m[k][j]-m[j][k])/s;q
    };
    Transform { position: p.map(|v|v as f32), rotation: q.map(|v|v as f32) }
}

fn limit(d: &LimitDynamics, angular: bool, mass: f32) -> (f32,f32,f32,f32,u32) {
    let soft=d.soft.unwrap_or(angular);
    let factor=if angular {mass*100000.0} else {1.};
    (d.restitution.unwrap_or(0.) as f32,
     d.contact_distance.unwrap_or(if angular {1.} else {5.}) as f32,
     d.stiffness.unwrap_or(if angular {50.} else {0.}) as f32*factor,
     d.damping.unwrap_or(if angular {5.} else {0.}) as f32*factor, u32::from(soft))
}

/// UE CreateConstraint 0x33170f0 reverses body/frame order. UE Swing1 maps to Px SWING2 and vice versa.
pub fn joint(c: &Constraint, parent: u32, child: u32, average_mass: f32) -> Joint {
    let l=&c.limits;let d=&c.dynamics;
    let vals=[limit(&d.linear_limit,false,average_mass),limit(&d.cone_limit,true,average_mass),limit(&d.twist_limit,true,average_mass)];
    let mut contact=vals.map(|v|v.1);
    if contact[1]>=1. {contact[1]=contact[1].min(0.49*l.swing1_deg.min(l.swing2_deg) as f32);}
    contact[1]=rad(contact[1] as f64);contact[2]=rad(contact[2] as f64);
    Joint { body0:parent,body1:child,frame0:frame(c.pos2,c.pri_axis2,c.sec_axis2),frame1:frame(c.pos1,c.pri_axis1,c.sec_axis1),
        motion:[motion(&l.linear_motion[0]),motion(&l.linear_motion[1]),motion(&l.linear_motion[2]),motion(&l.twist_motion),motion(&l.swing2_motion),motion(&l.swing1_motion)],
        linear_limit:l.linear_limit as f32,swing1:rad(l.swing2_deg.clamp(0.0001,179.999893)),swing2:rad(l.swing1_deg.clamp(0.0001,179.999893)),
        twist:rad(l.twist_deg),restitution:vals.map(|v|v.0),contact_distance:contact,stiffness:vals.map(|v|v.2),damping:vals.map(|v|v.3),soft:vals.map(|v|v.4),
        disable_collision:u32::from(l.disable_collision),projection:u32::from(d.projection_enabled.unwrap_or(true)),parent_dominates:u32::from(l.parent_dominates),
        projection_linear:d.projection_linear_cm.unwrap_or(5.) as f32,projection_angular:rad(d.projection_angular_deg.unwrap_or(180.)) }
}

pub fn death_sequence(angle: f32, random: u8, bone: &str, damage_type: i64, sub_type: i64) -> String {
    // PickDeathAnim @35594..37548: snapped 90-degree bins; spine stab special applies in bins0/±2.
    let bucket=(angle/90.).round() as i32;
    let name=if (bucket==0 || bucket.abs()==2) && bone.eq_ignore_ascii_case("Spine") && damage_type==1 && (sub_type==2 || sub_type==3) {
        "Death_Frontstomachhit"
    } else if bucket==0 { if random%2==0 {"Death_BackFront1"} else {"Death_0"} }
    else { ["Death_1","Death_2","Death_5","Death_BackFront1","Death_Frontfall1","Death_Frontfall2"][(random%6) as usize] };
    format!("{}{name}",if name.starts_with("Death_Front") || name=="Death_BackFront1" {RETARGET} else {RAW})
}

impl Ragdolls {
    pub fn open(rd: &Reader, bridge: &Path, dlls: &Path, gravity: f32) -> Result<Self,String> {
        let asset=mh_assets::physics::read(rd,ASSET).ok_or_else(||format!("No corpse physics asset {ASSET}"))?;
        // Original engine init0x2700260: length100 cm, speed1000 cm/s. Flesh material ctor0x26fcb20 0.7/0.3.
        let scene=Scene::open(bridge,dlls,gravity,0.7,0.3,100.,1000.)?;
        Ok(Self::with_scene(asset,scene))
    }
    /// Uses the exact production body/joint/death implementation in an isolated diagnostic process.
    /// This nondefault feature does not bypass production `Scene::open`.
    #[cfg(feature = "native-validation")]
    pub fn open_for_validation(rd: &Reader, bridge: &Path, dlls: &Path, gravity: f32) -> Result<Self,String> {
        let asset=mh_assets::physics::read(rd,ASSET).ok_or_else(||format!("No corpse physics asset {ASSET}"))?;
        let scene=Scene::open_for_validation(bridge,dlls,gravity,0.7,0.3,100.,1000.)?;
        Ok(Self::with_scene(asset,scene))
    }
    fn with_scene(asset: PhysicsAsset, scene: Scene) -> Self {
        Self {scene,asset,force_multipliers:Default::default(),force_ragdoll:Default::default(),corpses:Vec::new(),errors:Vec::new(),world_shapes:0,skipped_world_shapes:0}
    }
    pub fn remove(&mut self,fi: usize) {
        if let Some(i)=self.corpses.iter().position(|c|c.fighter==fi) {
            let c=self.corpses.remove(i);self.scene.remove_bodies(&c.bodies.iter().map(|b|b.1).collect::<Vec<_>>());
        }
    }
    /// Native D6 actor-frame separation, using the same descriptor conversion as creation.
    #[cfg(feature = "native-validation")]
    pub fn validation_joint_anchors(&self) -> Result<Vec<(String,f32,f32,bool)>,String> {
        let mut observations=Vec::new();
        for corpse in &self.corpses {
            if corpse.bodies.is_empty() { continue; }
            for c in &self.asset.constraints {
                let child=self.asset.bodies.iter().position(|b|b.bone.eq_ignore_ascii_case(&c.bone1)).ok_or("Validation child absent")?;
                let parent=self.asset.bodies.iter().position(|b|b.bone.eq_ignore_ascii_case(&c.bone2)).ok_or("Validation parent absent")?;
                let average=(self.asset.bodies[child].dynamics.mass_kg.ok_or("Validation child mass absent")?+
                             self.asset.bodies[parent].dynamics.mass_kg.ok_or("Validation parent mass absent")?) as f32*0.5;
                let d=joint(c,corpse.bodies[parent].1,corpse.bodies[child].1,average);
                let p=self.scene.pose(d.body0).ok_or("Native validation parent pose absent")?;
                let ch=self.scene.pose(d.body1).ok_or("Native validation child pose absent")?;
                let anchor0=engine(d.frame0).then(&engine(p)).loc;
                let anchor1=engine(d.frame1).then(&engine(ch)).loc;
                observations.push((format!("{}:{}->{}",corpse.fighter,c.bone1,c.bone2),
                                   (anchor0-anchor1).length(),d.projection_linear,d.projection!=0));
            }
        }
        Ok(observations)
    }
    fn bodies(&mut self,fi: usize,sk: &Skeleton,world: &[FTransform],velocity: FVector) -> Result<Vec<(usize,u32)>,String> {
        let mut ids=Vec::new();
        let result=(|| {
            for b in &self.asset.bodies {
                eprintln!("PHYSX create body {}",b.bone);
                let bone=sk.find(&b.bone).ok_or_else(||format!("Corpse bone absent: {}",b.bone))?;
                let mut shapes=Vec::new();
                for e in &b.geom.spheres {shapes.push(Shape {kind:0,local:local(e.center,[0.;3]),size:[e.radius as f32,0.,0.],rest_offset:e.shape.rest_offset as f32,contact_offset:offset(e.radius as f32)});}
                for e in &b.geom.boxes {shapes.push(Shape {kind:1,local:local(e.center,e.rotation_deg),size:[e.x as f32*0.5,e.y as f32*0.5,e.z as f32*0.5],rest_offset:e.shape.rest_offset as f32,contact_offset:offset(e.x.min(e.y).min(e.z) as f32*0.5)});}
                for e in &b.geom.sphyls {
                    let mut t=local(e.center,e.rotation_deg);
                    // Px capsule axisX → UE element axisZ; rigid-body transform remains in UE space.
                    let q=FQuat::from_array(t.rotation).mul(FQuat::from_array([0.,-std::f32::consts::FRAC_1_SQRT_2,0.,std::f32::consts::FRAC_1_SQRT_2]));t.rotation=[q.x,q.y,q.z,q.w];
                    shapes.push(Shape {kind:2,local:t,size:[e.radius as f32,e.length as f32*0.5,0.],rest_offset:e.shape.rest_offset as f32,contact_offset:offset(e.radius as f32)});
                }
                if !b.geom.convex.is_empty() || !b.geom.tapered.is_empty() {return Err(format!("Unsupported corpse geometry {}",b.bone));}
                let d=&b.dynamics;
                if d.override_mass!=Some(true) {return Err(format!("Unresolved calculated mass for {}",b.bone));}
                let sleep=50. * if d.sleep_family.as_deref().is_some_and(|s|s.ends_with("Custom")) {d.custom_sleep_threshold_multiplier.unwrap_or(1.) as f32} else {1.};
                let desc=Body {world:native(world[bone]),mass:d.mass_kg.ok_or_else(||format!("No stored mass {}",b.bone))? as f32,
                    linear_damping:d.linear_damping.unwrap_or(0.01) as f32,angular_damping:d.angular_damping.unwrap_or(0.) as f32,
                    com_nudge:d.com_nudge_cm.unwrap_or([0.;3]).map(|v|v as f32),sleep_threshold:sleep,
                    position_iterations:d.position_solver_iterations.unwrap_or(8) as u32,velocity_iterations:d.velocity_solver_iterations.unwrap_or(1) as u32,kinematic:0,
                    velocity:[velocity.x,velocity.y,velocity.z],group:fi as u32+1,max_angular_velocity:rad(3600.)};
                ids.push((bone,self.scene.add_body(&desc,&shapes)?));
            }
            let pairs:Vec<_>=self.asset.collision_disable_pairs.iter().map(|(p,_)|[ids[p[0]].1,ids[p[1]].1]).collect();self.scene.disable_pairs(&pairs);
            for c in &self.asset.constraints {
                eprintln!("PHYSX create joint {} -> {}",c.bone1,c.bone2);
                let ci=self.asset.bodies.iter().position(|b|b.bone.eq_ignore_ascii_case(&c.bone1)).ok_or("Joint child absent")?;
                let pi=self.asset.bodies.iter().position(|b|b.bone.eq_ignore_ascii_case(&c.bone2)).ok_or("Joint parent absent")?;
                let avg=(self.asset.bodies[ci].dynamics.mass_kg.unwrap()+self.asset.bodies[pi].dynamics.mass_kg.unwrap()) as f32*0.5;
                self.scene.add_joint(&joint(c,ids[pi].1,ids[ci].1,avg))?;
            }
            Ok(())
        })();
        if let Err(e)=result {self.scene.remove_bodies(&ids.iter().map(|b|b.1).collect::<Vec<_>>());return Err(e);}
        eprintln!("PHYSX corpse created");
        Ok(ids)
    }

    pub fn tick(&mut self,sim: &mut Sim,previous_velocity: &[FVector]) {
        let sk=&sim.geo.skeleton;
        for fi in 0..sim.combat.fighters.len() {
            if !sim.combat.fighters[fi].dead || self.corpses.iter().any(|c|c.fighter==fi) {continue;}
            let f=&sim.combat.fighters[fi];
            let hit=sim.combat.hits.iter().rev().find(|h|h.get("victim").and_then(|v|v.as_str())==Some(&f.name) && h.get("health").and_then(|v|v.as_i64()).is_some_and(|h|h<=0));
            let bone=hit.and_then(|h|h.get("bone")).and_then(|v|v.as_str()).unwrap_or("");
            let sub=hit.and_then(|h|h.get("move")).and_then(|v|v.as_i64()).unwrap_or(0);
            let typ=if hit.is_some_and(|h|h.get("ranged").and_then(|v|v.as_bool())==Some(true)) {2} else if hit.is_some() {1} else {0};
            let random=f.net.id as u8;
            let vel=previous_velocity.get(fi).copied().unwrap_or(FVector::ZERO);
            let leg=mordhau_core::combat::damage::is_leg(&sim.combat.spec.constants,bone);
            let attacker=hit.and_then(|h|h.get("attacker")).and_then(|v|v.as_str()).and_then(|n|sim.combat.fighters.iter().position(|a|a.name==n));
            let direction=attacker.and_then(|a|sim.posed.borrow().get(&sim.combat.fighters[a].name).map(|p|p.tracer.last_observed_direction)).unwrap_or_default();
            let (sy,cy)=sim.yaw[fi].to_radians().sin_cos();
            let angle=(direction.y*cy-direction.x*sy).atan2(direction.x*cy+direction.y*sy).to_degrees();
            let forced=attacker.is_some_and(|a|self.force_ragdoll.contains(&sim.combat.fighters[a].weapon_path));
            let immediate=f.airborne || random%10<5 || leg || (typ==1 && sub==6) || vel.length()>=100. || forced;
            let sequence=death_sequence(angle,random,bone,typ,sub);
            let initial_world=if let Some(p)=sim.posed.borrow().get(&f.name) {(0..p.bones.len()).map(|b|sim.geo.bone_world(p,b)).collect()} else {Vec::new()};
            let killing_impulse=attacker.and_then(|a| {
                if !immediate || typ!=1 {return None;}
                let multiplier=self.force_multipliers.get(&sim.combat.fighters[a].weapon_path).copied().unwrap_or(3.5);
                let magnitude=(10000.*multiplier*(0.2+random as f32/255.*0.8)).min(30000.*if sub==2 || sub==3 {0.75} else {1.}) * if sub==6 {3.} else {1.};
                let mut force=direction.scale(magnitude as f64);
                let inherited=FVector::new(vel.x,vel.y,vel.z.clamp(-500.,0.));
                if inherited.dot(force).acos().to_degrees()<90. && force.dot(force)>0. {
                    let projected=force.scale((inherited.scale(15.).dot(force)/force.dot(force)) as f64);
                    force=force-clamp_size(projected,force.length());
                }
                force=clamp_size(force,15000.);
                let point=hit.and_then(|h|h.get("impact")).and_then(|v|v.as_array()).filter(|v|v.len()==3)
                    .map(|v|FVector::new(v[0].as_f64().unwrap_or(0.) as f32,v[1].as_f64().unwrap_or(0.) as f32,v[2].as_f64().unwrap_or(0.) as f32))?;
                Some((bone.to_string(),force,point))
            });
            self.corpses.push(Corpse {fighter:fi,sequence,sequence_time:0.,started:sim.combat.now,blend_started:immediate.then_some(sim.combat.now),bodies:Vec::new(),weight:if immediate {1.} else {0.},initial_world,killing_impulse,impulse_applied:false});
        }
        // Sample each death sequence; its BeginRagdoll notify time is a sequence position, never a wall-clock delay.
        for i in 0..self.corpses.len() {
            let fi=self.corpses[i].fighter;
            if !sim.combat.fighters[fi].dead {continue;}
            let old=self.corpses[i].sequence_time;
            if self.corpses[i].blend_started.is_none_or(|start|sim.combat.now-start<3.) {
                self.corpses[i].sequence_time += sim.dt as f64 * (1.-0.5*sim.fanim[fi].proc.is_first_person as f64);
            }
            let t=self.corpses[i].sequence_time;
            if let Some(a)=&sim.anim {
                if let Some(p)=sim.posed.borrow_mut().get_mut(&sim.combat.fighters[fi].name) {
                    let local=a.sample(sk,&self.corpses[i].sequence,t,false);
                    p.bones=sk.to_component(&local);
                    sim.fanim[fi].local=local;
                    sim.fanim[fi].last_cs=p.bones.clone();
                }
                if self.corpses[i].blend_started.is_none() && a.notifies(&self.corpses[i].sequence).iter().any(|(n,at)|n=="BeginRagdoll" && *at>=old && *at<=t) {
                    self.corpses[i].blend_started=Some(sim.combat.now);self.corpses[i].weight=0.001;
                }
            }
            if self.corpses[i].blend_started.is_some() && self.corpses[i].bodies.is_empty() && self.errors.is_empty() {
                let world=if self.corpses[i].weight==1. { self.corpses[i].initial_world.clone() } else if let Some(p)=sim.posed.borrow().get(&sim.combat.fighters[fi].name) { (0..p.bones.len()).map(|b|sim.geo.bone_world(p,b)).collect::<Vec<_>>() } else {continue};
                let v=if self.corpses[i].weight==1. {previous_velocity.get(fi).copied().unwrap_or(FVector::ZERO)} else {FVector::ZERO};
                // ApplyRagdollForce clamps inherited upward velocity away, downward to -500 (BP @5090).
                match self.bodies(fi,sk,&world,FVector::new(v.x,v.y,v.z.clamp(-500.,0.))) {
                    Ok(ids)=> {
                        if let Some((ref bone,force,point))=self.corpses[i].killing_impulse {
                            let exact=self.asset.bodies.iter().position(|b|b.bone.eq_ignore_ascii_case(bone));
                            let index=exact.or_else(||self.asset.bodies.iter().position(|b|b.bone=="Spine1"));
                            if let Some(body)=index {
                                let mass=self.asset.bodies[body].dynamics.mass_kg.unwrap() as f32;
                                let amount=force.scale((if exact.is_some() {2.5} else {2.})*(mass/15.).max(0.5) as f64);
                                match self.scene.impulse_at(ids[body].1,[amount.x,amount.y,amount.z],[point.x,point.y,point.z]) {
                                    Ok(())=>self.corpses[i].impulse_applied=true,
                                    Err(e)=>self.errors.push(e),
                                }
                            }
                        }
                        self.corpses[i].bodies=ids;
                    },
                    Err(e)=>{eprintln!("Corpse physics: {e}");self.errors.push(e);}
                }
            }
        }
        if self.corpses.iter().any(|c|!c.bodies.is_empty()) {
            // DefaultEngine.ini substepping: max physics dt 0.033333, substep 0.016667, max6.
            let dt=sim.dt.min(0.033333);let steps=(dt/0.016667).ceil().clamp(1.,6.) as u32;
            for _ in 0..steps {if let Err(e)=self.scene.step(dt/steps as f32) {self.errors.push(e);break;}}
        }
        for c in &mut self.corpses {
            if c.bodies.is_empty() {continue;}
            if c.weight<1. {let a=((sim.combat.now-c.blend_started.unwrap())/0.2) as f32;c.weight=(a.clamp(0.,1.)*a.clamp(0.,1.)*(3.-2.*a.clamp(0.,1.))+0.001).min(1.);}
            let mut posed=sim.posed.borrow_mut();let Some(p)=posed.get_mut(&sim.combat.fighters[c.fighter].name) else {continue};
            let local=sk.to_local(&p.bones);let mesh=sim.geo.mesh_xf_adj(p.mesh_z_adjust).then(&p.actor);let inv=mesh.inverse();
            for b in 0..p.bones.len() {
                let animated=if sk.parents[b]<0 {local[b]} else {local[b].then(&p.bones[sk.parents[b] as usize])};
                p.bones[b]=if let Some((_,id))=c.bodies.iter().find(|(bone,_)|*bone==b) {
                    self.scene.pose(*id).map(|t|crate::procedural::blend_with(animated,engine(t).then(&inv),c.weight)).unwrap_or(animated)
                } else {animated};
            }
            sim.fanim[c.fighter].last_cs=p.bones.clone();
            sim.fanim[c.fighter].local=sk.to_local(&p.bones);
        }
    }
    pub fn debug(&self) -> serde_json::Value {
        serde_json::json!({"backend":"PhysX3.4.0", "world_shapes":self.world_shapes,"skipped_world_shapes":self.skipped_world_shapes,"errors":self.errors,
            "corpses":self.corpses.iter().map(|c|serde_json::json!({"fighter":c.fighter,"sequence":c.sequence,"sequence_time":c.sequence_time,"started":c.started,"blend_started":c.blend_started,"weight":c.weight,"bodies":c.bodies.len(),"impulse_applied":c.impulse_applied})).collect::<Vec<_>>()})
    }
}

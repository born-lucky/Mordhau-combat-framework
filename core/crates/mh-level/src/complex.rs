//! ORIGINAL cooked PhysX complex WorldStatic object queries for static mesh placements.
//! First slice: triangle meshes, per-face materials and exact StaticMeshActor metadata.
//! Landscape/splines/simple-as-complex/masked materials/unknown actor defaults are reported, never substituted.
//! Registration and production native activation await the guarded original-wall fixture.
use crate::{collision::CollisionWorld, level::Pkgs, LevelData, MeshPlacement, Xf};
use mh_physics::{Scene, Transform, cooked::{CookedInfo,CookedRayHit}};
use serde_json::Value;
use std::{cell::RefCell, collections::{BTreeMap, HashMap}};

#[derive(Clone, Debug)]
pub struct ComplexHit {
    pub component: u32,
    pub actor_path: String,
    pub can_be_damaged: bool,
    pub surface: u8,
    pub face: u32,
    pub material: u32,
    pub distance: f32,
    pub position: [f32;3],
    pub normal: [f32;3],
}
struct Placed {
    handle:u32, component:u32, actor_path:String, can_be_damaged:bool,
    placement:Transform, scale:[f32;3], double_sided:bool, surfaces:Vec<u8>, minimum:[f64;3], maximum:[f64;3],
}
pub struct ComplexTraceWorld {
    scene:Scene,
    geometries:Vec<Placed>,
    pub skipped:BTreeMap<String,usize>,
    /// Preserve original actor/component/cooked-face identity for canonical QA, separate from render material sections.
    pub last_hit:RefCell<Option<ComplexHit>>,
}

fn count(skipped:&mut BTreeMap<String,usize>, why:impl Into<String>) { *skipped.entry(why.into()).or_default()+=1; }
fn object_path(v:Option<&Value>)->&str { v.and_then(|v|v.get("ObjectPath")).and_then(Value::as_str).unwrap_or("") }
fn eligible(body:&crate::collision::Body)->bool { body.queries() && body.object_type=="WorldStatic" }

/// Exact affine representation only. A shear is not silently discarded as if it were a UE FTransform.
fn placement(x:Xf)->Result<(Transform,[f32;3]),String> {
    if x.m.iter().flatten().any(|v|!v.is_finite()) { return Err("nonfinite placement".into()); }
    let mut scale=[0.;3];let mut rot=[[0.;3];3];
    for c in 0..3 {
        scale[c]=(0..3).map(|r|x.m[r][c]*x.m[r][c]).sum::<f64>().sqrt();
        if scale[c]==0. { return Err("singular placement".into()); }
        for r in 0..3 { rot[r][c]=x.m[r][c]/scale[c]; }
    }
    for c in 0..3 { for d in c+1..3 {
        if (0..3).map(|r|rot[r][c]*rot[r][d]).sum::<f64>().abs()>1e-10 { return Err("sheared placement".into()); }
    } }
    let det=rot[0][0]*(rot[1][1]*rot[2][2]-rot[1][2]*rot[2][1])-rot[0][1]*(rot[1][0]*rot[2][2]-rot[1][2]*rot[2][0])+rot[0][2]*(rot[1][0]*rot[2][1]-rot[1][1]*rot[2][0]);
    if det<0. { return Err("negative-scale placement awaits original transform review".into()); }
    let trace=rot[0][0]+rot[1][1]+rot[2][2];
    let q = if trace>0. {
        let t=(trace+1.).sqrt()*2.;[(rot[2][1]-rot[1][2])/t,(rot[0][2]-rot[2][0])/t,(rot[1][0]-rot[0][1])/t,t/4.]
    } else {
        let i=if rot[0][0]>rot[1][1] && rot[0][0]>rot[2][2] {0} else if rot[1][1]>rot[2][2] {1} else {2};
        let j=(i+1)%3;let k=(i+2)%3;let t=(1.+rot[i][i]-rot[j][j]-rot[k][k]).sqrt()*2.;let mut q=[0.;4];
        q[i]=t/4.;q[j]=(rot[j][i]+rot[i][j])/t;q[k]=(rot[k][i]+rot[i][k])/t;q[3]=(rot[k][j]-rot[j][k])/t;q
    };
    let p=Transform {position:x.translation().map(|v|v as f32),rotation:q.map(|v|v as f32)};
    let scale=scale.map(|v|v as f32);
    if p.position.iter().chain(&p.rotation).chain(&scale).any(|v|!v.is_finite()) { return Err("placement exceeds original binary32 range".into()); }
    Ok((p,scale))
}

/// Rotation-independent conservative world cube. Native local bounds are used, never the movement hull.
/// The L1 radius contains every rotated vertex. Outward rounding covers binary32 pose normalization/query arithmetic;
/// broad phase may admit extra candidates but does not change the native narrow-phase hit.
fn bounds(info:&CookedInfo,p:&Transform,scale:[f32;3])->([f64;3],[f64;3]) {
    let radius=(0..3).map(|i|info.minimum[i].abs().max(info.maximum[i].abs()) as f64*scale[i].abs() as f64).sum::<f64>();
    let radius=radius*(1.+64.*f32::EPSILON as f64)+f32::MIN_POSITIVE as f64;
    (p.position.map(|v|v as f64-radius),p.position.map(|v|v as f64+radius))
}
fn overlaps_segment(min:[f64;3],max:[f64;3],a:[f32;3],b:[f32;3])->bool {
    (0..3).all(|i|a[i].min(b[i]) as f64<=max[i] && a[i].max(b[i]) as f64>=min[i])
}
fn decode_surface(value:&Value)->Result<u8,String> {
    let name=value.as_str().ok_or("undecodable complex PhysicalMaterial SurfaceType")?;
    let name=name.strip_prefix("EPhysicalSurface::").unwrap_or(name);
    if name=="SurfaceType_Default" {return Ok(0)}
    let digits=name.strip_prefix("SurfaceType").filter(|v|!v.is_empty() && v.bytes().all(|b|b.is_ascii_digit()))
        .ok_or("undecodable complex PhysicalMaterial SurfaceType")?;
    digits.parse::<u8>().map_err(|_|"undecodable complex PhysicalMaterial SurfaceType".into())
}

fn material_surface(pk:&Pkgs,path:&str,cache:&mut HashMap<String,u8>)->Result<u8,String> {
    let mut path=path.to_string();let mut seen=Vec::new();let mut physical=None;
    for _ in 0..32 {
        if path.is_empty() { break; }
        if seen.contains(&path) { return Err("material parent cycle".into()); }seen.push(path.clone());
        let object=pk.obj(Some(&serde_json::json!({"ObjectPath":path}))).ok_or("missing complex material")?;
        let ty=object.get("Type").and_then(Value::as_str).unwrap_or("");
        if !matches!(ty,"Material"|"MaterialInstanceConstant") { return Err("dynamic/unsupported complex material".into()); }
        let props=pk.props(&object);
        if !object_path(props.get("PhysMaterialMask")).is_empty() || props.get("PhysMaterialMaskMap").and_then(Value::as_array).is_some_and(|a|!a.is_empty()) {
            return Err("physical material mask query is unported".into());
        }
        if physical.is_none() { let pm=object_path(props.get("PhysMaterial"));if !pm.is_empty() {physical=Some(pm.to_string());} }
        path=object_path(props.get("Parent")).to_string();
    }
    if !path.is_empty() {return Err("material parent chain exceeds reviewed traversal bound".into())}
    let Some(physical)=physical else {return Ok(0)}; // Original GEngine DefaultPhysicalMaterial fallback.
    if let Some(&surface)=cache.get(&physical) {return Ok(surface)}
    let object=pk.obj(Some(&serde_json::json!({"ObjectPath":physical}))).ok_or("missing exact complex PhysicalMaterial export")?;
    if object.get("Type").and_then(Value::as_str)!=Some("PhysicalMaterial") {return Err("unsupported complex PhysicalMaterial class".into())}
    let props=pk.props(&object);
    if props.contains_key("__unsupported__") {return Err("undecodable complex PhysicalMaterial export".into())}
    let surface=match props.get("SurfaceType") {
        None=>0,
        Some(value)=>decode_surface(value)?,
    };
    cache.insert(physical,surface);Ok(surface)
}
fn surfaces(pk:&Pkgs,m:&MeshPlacement,cache:&mut HashMap<String,u8>)->Result<Vec<u8>,String> {
    let mesh=pk.obj(Some(&serde_json::json!({"ObjectPath":m.mesh}))).ok_or("missing exact StaticMesh material owner")?;
    let props=pk.props(&mesh);let base=props.get("StaticMaterials").and_then(Value::as_array).cloned().unwrap_or_default();
    let n=base.len().max(m.material_paths.len());let mut out=Vec::with_capacity(n);
    for i in 0..n {
        let override_path=m.material_paths.get(i).filter(|s|!s.is_empty()).map(String::as_str);
        let path=override_path.unwrap_or_else(||object_path(base.get(i).and_then(|m|m.get("MaterialInterface"))));
        out.push(material_surface(pk,path,cache)?);
    }
    Ok(out)
}

impl ComplexTraceWorld {
    /// Native Scene creation belongs to the host. Pending cooked activation rejects loading before deserialization.
    /// CollisionWorld supplies only independently reconstructed body settings; its simple geometry is NEVER queried here.
    pub fn build(pk:&Pkgs,d:&LevelData,settings:&CollisionWorld,mut scene:Scene)->Result<Self,String> {
        let mut geometries=Vec::new();let mut skipped=BTreeMap::new();let mut materials=HashMap::new();
        let bodies:HashMap<_,_>=settings.bodies.iter().enumerate().map(|(i,b)|(b.source.as_str(),(i,b))).collect();
        let mut meshes:HashMap<String,(Vec<(u32,CookedInfo)>,bool)>=HashMap::new();
        if !d.landscape_collision.is_empty() { skipped.insert("landscape complex query is unported".into(),d.landscape_collision.len()); }
        if !d.splines.is_empty() { skipped.insert("spline cooked query is unported".into(),d.splines.len()); }
        for m in &d.meshes {
            let Some(&(component,body))=bodies.get(m.component_path.as_str()) else {count(&mut skipped,"no reconstructed collision body");continue};
            if !eligible(body) {continue}
            let comp=pk.obj(Some(&serde_json::json!({"ObjectPath":m.component_path}))).ok_or("missing collision component")?;
            let owner=pk.obj(comp.get("Outer")).ok_or("missing static collision actor")?;
            let actor_path=object_path(comp.get("Outer")).to_string();let owner_props=pk.props(&owner);
            let can_be_damaged=match owner_props.get("bCanBeDamaged").and_then(Value::as_bool) {
                Some(v)=>v,None if owner.get("Type").and_then(Value::as_str)==Some("StaticMeshActor")=>false,
                None=>{count(&mut skipped,"unreviewed actor CanBeDamaged default");continue},
            };
            let slots=match surfaces(pk,m,&mut materials) {Ok(v)=>v,Err(e)=>{count(&mut skipped,e);continue}};
            if !meshes.contains_key(&m.mesh) {
                let cooked=match mh_pak::cooked_body::read(&pk.rd,&m.mesh) {Ok(v)=>v,Err(e)=>{count(&mut skipped,e);continue}};
                let mode=cooked.properties.get("CollisionTraceFlag").and_then(Value::as_str).unwrap_or("CTF_UseDefault");
                let default=crate::config::value(&pk.rd.vfs,"DefaultEngine.ini","/Script/Engine.PhysicsSettings","DefaultShapeComplexity");
                let effective=if mode.ends_with("UseDefault") {default.as_str()} else {mode};
                if !(effective.ends_with("UseSimpleAndComplex") || effective.ends_with("UseComplexAsSimple")) {
                    count(&mut skipped,format!("unsupported complex collision mode {effective}"));continue;
                }
                let set=scene.load_cooked(&cooked.payload,cooked.payload.len(),cooked.sha1)?;
                let mut handles=Vec::new();for handle in set.triangle_handles() {handles.push((handle,scene.cooked_info(handle)?));}
                if handles.is_empty() {count(&mut skipped,"no original cooked triangles");continue}
                let double_sided=cooked.properties.get("bDoubleSidedGeometry").and_then(Value::as_bool).unwrap_or(false);
                meshes.insert(m.mesh.clone(),(handles,double_sided));
            }
            let (handles,double_sided)=meshes.get(&m.mesh).unwrap();
            let transforms:Vec<_>=if m.instances.is_empty() {vec![m.xf]} else {m.instances.iter().map(|i|m.xf * *i).collect()};
            for transform in transforms {
                let (placement,scale)=match placement(transform) {Ok(v)=>v,Err(e)=>{count(&mut skipped,e);continue}};
                for &(handle,info) in handles {
                    let (minimum,maximum)=bounds(&info,&placement,scale);
                    geometries.push(Placed {handle,component:component as u32,actor_path:actor_path.clone(),can_be_damaged,
                        placement,scale,double_sided:*double_sided,surfaces:slots.clone(),minimum,maximum});
                }
            }
        }
        Ok(Self {scene,geometries,skipped,last_hit:RefCell::new(None)})
    }
    pub fn geometry_count(&self)->usize {self.geometries.len()}
    pub fn ray(&self,a:[f32;3],b:[f32;3])->Result<Option<ComplexHit>,String> {
        *self.last_hit.borrow_mut()=None;
        let delta=std::array::from_fn::<_,3,_>(|i|b[i]-a[i]);let distance=delta.iter().map(|v|v*v).sum::<f32>().sqrt();
        if distance==0. {return Ok(None)}
        if !distance.is_finite() {return Err("nonfinite complex query segment".into())}
        let direction=delta.map(|v|v/distance);let mut best:Option<(&Placed,CookedRayHit)>=None;
        for g in &self.geometries {
            if !overlaps_segment(g.minimum,g.maximum,a,b) {continue}
            let Some(hit)=self.scene.ray_cooked(g.handle,&g.placement,g.scale,g.double_sided,a,direction,distance)? else {continue};
            if best.as_ref().is_some_and(|(_,old)|old.distance<=hit.distance) {continue}
            best=Some((g,hit));
        }
        // Original single-query conversion consumes the winning hit, not every temporary broadphase candidate.
        // Unsupported material metadata on a farther candidate cannot override a valid nearer collision.
        let hit=match best {
            None=>None,
            Some((g,hit))=>{
                if hit.material==u16::MAX as u32 {return Err("native missing triangle-material sentinel is outside reviewed scope".into())}
                let surface=*g.surfaces.get(hit.material as usize).ok_or("native cooked material slot exceeds original component materials")?;
                Some(ComplexHit {component:g.component,actor_path:g.actor_path.clone(),can_be_damaged:g.can_be_damaged,surface,
                    face:hit.face,material:hit.material,distance:hit.distance,position:hit.position,normal:hit.normal})
            },
        };
        *self.last_hit.borrow_mut()=hit.clone();Ok(hit)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_surface_default_is_an_exact_enum_value_not_an_error_fallback() {
        for value in ["SurfaceType_Default","EPhysicalSurface::SurfaceType_Default"] {assert_eq!(decode_surface(&Value::from(value)).unwrap(),0);}
        assert_eq!(decode_surface(&Value::from("EPhysicalSurface::SurfaceType2")).unwrap(),2);
        for value in ["wrongSurfaceType2","SurfaceType_unknown","SurfaceType","Other::SurfaceType2","SurfaceType999"] {
            assert!(decode_surface(&Value::from(value)).is_err());
        }
        assert!(decode_surface(&Value::Null).is_err());
    }
    #[test]
    fn original_object_mask_ignores_pawn_channel_responses_but_requires_queries() {
        let mut body=crate::collision::Body {name:String::new(),kind:"mesh",source:String::new(),mesh:String::new(),profile:String::new(),
            collision_enabled:"QueryOnly".into(),object_type:"WorldStatic".into(),responses:HashMap::from([("Pawn".into(),crate::collision::Resp::Ignore)]),
            can_step_up:false,physical_materials:Vec::new(),surfaces:Vec::new()};
        assert!(eligible(&body),"Original object-mask ray is independent of Pawn channel response");
        body.collision_enabled="PhysicsOnly".into();assert!(!eligible(&body));
        body.collision_enabled="NoCollision".into();assert!(!eligible(&body));
        body.collision_enabled="QueryAndPhysics".into();body.object_type="WorldDynamic".into();assert!(!eligible(&body));
        body.object_type="WorldStatic".into();assert!(eligible(&body));
    }
    #[test]
    fn placement_keeps_rotation_scale_and_rejects_unreviewed_transforms() {
        let q=mh_pak::aggeom::rotator_quat([23.,-47.,11.]);let x=Xf::trs([42.,-17.,9.],q,[2.,3.,4.]);
        let (p,s)=placement(x).unwrap();
        let reconstructed=Xf::trs(p.position.map(|v|v as f64),p.rotation.map(|v|v as f64),s.map(|v|v as f64));
        assert!(x.max_diff(&reconstructed)<1e-6,"Pure authored TRS must survive the native descriptor conversion");
        let mut shear=x;shear.m[0][1]+=0.5;assert!(placement(shear).is_err());
        assert!(placement(Xf::trs([0.;3],[0.,0.,0.,1.],[-1.,1.,1.])).is_err());
        assert!(placement(Xf::trs([0.;3],[0.,0.,0.,1.],[0.,1.,1.])).is_err());
    }
}

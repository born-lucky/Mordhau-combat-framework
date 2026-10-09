//! The exe's weapon trace in UE space, as a mordhau-core `TraceHost`: AMordhauWeapon's tracer state per fighter
//! (PrepareForTracing rva=0x16378e0, ResetTracers rva=0x163bb80, GetTrace_Implementation rva=0x1629520) on the posed
//! weapon, and SampleTracers rva=0x163c430's sample segments against every other character's physics-asset bodies.
//! The SampleTracer rules (nearest first, ignored actors, parrying hands) and the hit processing stay in mordhau-core.
//!
//! SampleTracers in binary32 (non-cosmetic path): dirC = (CurStart - CurEnd).GetSafeNormal(), dirP likewise,
//! n = RoundToInt(|CurEnd - CurStart| * 2/7.5 ...): (int)ROUND(len * tracer_count_scale + tracer_round_bias) >> 1, then
//! for i = n .. 0 step tracer_step (-1): segment PrevEnd + dirP * i * 7.5 -> CurEnd + dirC * i * 7.5 (UE cm).
//! Four extra hand/environment segments precede the ordinary segments when the authored weapon flag is set,
//! except moves4/5. They accept clashes and WorldStatic, excluding character/body/block/shield hits. The original
//! complex WorldStatic query latches the first segment hit separately from channel0xf pawn/equipment hits.
//! Kick primary/additional traces use UKickMotion's character sockets. Additional
//! shield tracers and cosmetic network tracing remain separate unported paths.

use crate::physics::{segment_shape, BodyShape};
use mordhau_core::combat::world::{HitComp, RawHit, TraceHost, TraceSample, WorldBlock};
use mordhau_core::combat::World;
use mordhau_core::ue::{cvtss2si, ue_safe_normal, FTransform, FVector};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// AMordhauWeapon tracer fields (types/AMordhauWeapon.h)
#[derive(Clone, Debug, Default)]
pub struct UeTracer {
    pub cur_start: FVector,  // +0xd48
    pub cur_end: FVector,    // +0xd54
    pub prev_start: FVector, // +0xd60
    pub prev_end: FVector,   // +0xd6c
    pub cur_valid: bool,     // +0xd40
    pub prev_valid: bool,    // +0xd41
    pub invalidated: bool,   // +0xd42
    pub last_observed_direction: FVector, // +0x1c80; PrepareForTracing 0x16378e0
}

/// A held weapon's geometry: trace sockets on its root bone and its grip (EquipmentDef fields, UE units)
#[derive(Clone, Debug, Default)]
pub struct WeaponGeo {
    pub trace_start: Option<FVector>,
    pub trace_end: Option<FVector>,
    /// ClashNormal / SecondClashNormal and the ClashCapsuleBP radius (physics::weapon_clash_ue)
    pub clash_normal: FVector,
    pub second_clash_normal: FVector,
    pub clash_radius: f32,
    /// AMordhauWeapon ParryBoxTransform +0x19c0 (class defaults; absent: identity)
    pub parry_box: Option<mordhau_core::combat::geometry::ScaledXf>,
    /// SecondTraceStart / SecondTraceEnd (alternate mode)
    pub second_trace_start: Option<FVector>,
    pub second_trace_end: Option<FVector>,
    pub right_hand_equip_offset: FVector,
    pub rotation_offset: [f32; 3],
    pub grip_location_local: FVector,
    pub right_handed: bool,
    /// AMordhauEquipment EquippedOffset when bUseEquippedOffset (shields, throwables, tools): ComputeGrippedTransform
    /// then attaches by this transform alone (first-person r1)
    pub equipped_offset: Option<FTransform>,
    /// a shield's BlockCollider box: (relative location, half extents) in the weapon actor's frame
    /// (physics::weapon_block_box); None for weapons without one
    pub block_box: Option<(FVector, FVector)>,
    /// grip r2: the class path (matches Posed::grip.weapon), both modes' grip fields (grip.rs; None: the primary
    /// fields above only) and the raw EquippedOffset (used by whichever mode sets b(Second)UseEquippedOffset)
    /// Native AMordhauWeapon ctor true; FistsWeapon/KickWeapon ctor false; inherited cooked bool wins.
    pub extra_environment_tracers: bool,
    pub path: String,
    pub grip_modes: Option<[crate::grip::GripMode; 2]>,
    pub equipped_offset_xf: FTransform,
}

/// What every fighter shares: body shapes, the skeleton's bone indices of their bones, mesh placement, constants
pub struct Geometry {
    pub shapes: Vec<BodyShape>,
    pub shape_bones: Vec<Option<usize>>,
    pub skeleton: crate::pose::Skeleton,
    pub mesh_xf: FTransform,
    pub character_sockets: HashMap<String, (usize, FTransform)>,
    /// BP_MordhauCharacter BlockCollider template: RelativeLocation and BoxExtent (UE cm)
    pub block_collider_rel: FVector,
    pub block_collider_extent: FVector,
    /// Original / Low / HighBlockColliderRelativeOffset (physics::block_collider_offsets)
    pub block_collider_offsets: [mordhau_core::combat::geometry::ScaledXf; 3],
    pub weapons: HashMap<String, WeaponGeo>,
    pub grip_pitch_right: f32,
    pub grip_pitch_left: f32,
    pub count_scale: f32,
    pub round_bias: f32,
    pub spacing_cm: f32,
    pub step: f32,
}

/// One fighter's posed state, fed by the sim each frame (actor transform, component-space bones)
#[derive(Clone, Debug, Default)]
pub struct Posed {
    pub actor: FTransform,
    /// EVD_CAM_021: the crouch's HalfHeightAdjust (ExeMovement::half_height_adjust), added to the mesh's relative Z
    pub mesh_z_adjust: f32,
    pub bones: Vec<FTransform>,
    pub tracer: UeTracer,
    pub additional_tracer: Option<UeTracer>,
    /// the ClashCollider capsule relative to the weapon mesh (centre, half height incl. radius, radius), placed by
    /// RepositionClashCollider at the attack's start; None before any attack
    pub clash: Option<(FVector, f32, f32)>,
    /// grip r2: the held weapon's mode and a running mode switch's mesh override (grip.rs)
    pub grip: crate::grip::GripPosed,
}

/// Fresh complex WorldStatic object query, separate from the movement Pawn-channel world.
pub type WorldStaticQuery = Rc<dyn Fn(FVector, FVector) -> Option<WorldBlock>>;

pub struct SimTrace {
    pub geo: Rc<Geometry>,
    /// fighter name -> pose
    pub posed: Rc<RefCell<HashMap<String, Posed>>>,
    /// SampleTracers 0x163c430: publish the actual swept point segments, including misses.
    pub sampled: Rc<RefCell<Vec<SampledTrace>>>,
    pub world_static: Rc<RefCell<Option<WorldStaticQuery>>>,
}

#[derive(Clone, Debug)]
pub struct SampledTrace {
    pub fighter: usize,
    pub tick: u64,
    pub start: FVector,
    pub end: FVector,
    /// The four existing primary hand/environment sweeps, not attack phase or damage coverage.
    pub environment_only: bool,
}

impl Geometry {
    /// UKickMotion OverrideTrace/OverrideAdditionalTrace: animated socket -> mesh -> actor.
    pub fn character_socket_world(&self, p: &Posed, name: &str) -> FVector {
        self.character_sockets.get(name).filter(|(bone, _)| *bone < p.bones.len())
            .map(|(bone, socket)| socket.then(&self.bone_world(p, *bone)).loc)
            .unwrap_or_else(|| self.mesh_xf_adj(p.mesh_z_adjust).then(&p.actor).loc)
    }
    pub fn bone_world(&self, p: &Posed, bone: usize) -> FTransform {
        p.bones[bone].then(&self.mesh_xf_adj(p.mesh_z_adjust)).then(&p.actor)
    }
    /// EVD_CAM_021: the mesh relative to the actor with the crouch's HalfHeightAdjust on its Z (OnStartCrouch /
    /// OnEndCrouch: RelativeLocation.Z = the class default's -97 + HalfHeightAdjust)
    pub fn mesh_xf_adj(&self, z_adjust: f32) -> FTransform {
        let mut x = self.mesh_xf;
        x.loc.z += z_adjust;
        x
    }
    /// the held weapon's transform (pose.rs gripped on RightWeapon / LeftWeapon; held_weapon.gd socket_name).
    /// UEquipmentSystemComponent::ComputeGrippedTransform rva=0x14b70f0 (decomp UEquipmentSystemComponent.cpp 74-202):
    /// bIsLeft (the LeftHand / LeftWeapon socket, rva=0x14b7990) negates the RotationOffset's pitch / yaw / roll;
    /// bUseEquippedOffset (decomp 208-447) = FTransform::Multiply(EquippedOffset, socket). grip r2 (grip.rs): the
    /// fields of the fighter's current weapon mode (SwitchMode_Implementation rva=0x156e280 swaps the Second* ones
    /// in), and while a mode switch runs, the weapon mesh's world transform from PerformVirtualReparentTrickery
    /// rva=0x166a520 (the mesh, which the traces and the drawing follow, not the actor).
    pub fn weapon_world(&self, p: &Posed, weapon: &WeaponGeo) -> Option<FTransform> {
        let mine = !weapon.path.is_empty() && p.grip.weapon == weapon.path;
        if mine {
            if let Some(x) = p.grip.mesh_world {
                return Some(x);
            }
        }
        crate::grip::steady(self, p, weapon, mine && p.grip.alt)
    }
}

impl SimTrace {
    fn is_kick(&self, w: &World, fi: usize) -> bool {
        w.cur_m(fi).is_some_and(|m| m.attack().is_some() && w.spec.is_class_of(&m.def.base.native, "UKickMotion"))
    }
    fn attack_weapon<'a>(&self, w: &'a World, fi: usize) -> Option<&WeaponGeo> {
        match w.cur_m(fi).filter(|m| m.attack().is_some()) {
            Some(m) => {
                // reset runs before OnBegin assigns the capture; resolve the same FindWeapon actor then.
                let id = m.attack().and_then(|a| a.weapon_actor).or_else(|| w.find_weapon_actor(fi, &m.def.base.native))?;
                self.geo.weapons.get(&w.equipment_actor(id)?.path)
            }
            None => self.weapon_of(w, fi),
        }
    }
    fn weapon_of<'w>(&self, w: &'w World, fi: usize) -> Option<&WeaponGeo> {
        let f = &w.fighters[fi];
        f.weapon.as_ref()?;
        self.geo.weapons.get(&f.weapon_path)
    }
}

impl TraceHost for SimTrace {
    /// AMordhauWeapon::PrepareForTracing rva=0x16378e0: Previous = Current; Current = GetTrace (sockets now);
    /// bArePreviousTracersValid = bAreCurrentTracersValid; bAreCurrentTracersValid = !bAreCurrentTracersInvalidated
    fn prepare(&self, w: &World, fi: usize) {
        let name = &w.fighters[fi].name;
        let kick = self.is_kick(w, fi);
        let wg = self.attack_weapon(w, fi).cloned();
        let mut posed = self.posed.borrow_mut();
        let Some(p) = posed.get_mut(name) else { return };
        let (cs, ce) = if kick {
            (self.geo.character_socket_world(p, "KickTracerStart"), self.geo.character_socket_world(p, "KickTracerEnd"))
        } else { match wg.as_ref().and_then(|g| self.geo.weapon_world(p, g).map(|x| (x, g))) {
            // GetTrace_Implementation rva=0x1629520: the Second* sockets while bIsUsingAlternateMode; a missing socket
            // gives the component's own location (USkinnedMeshComponent::GetSocketLocation fallback: engine
            // behaviour, not disassembled)
            Some((wx, g)) => {
                let (s, e) = if w.fighters[fi].alternate_mode { (g.second_trace_start, g.second_trace_end) } else { (g.trace_start, g.trace_end) };
                (wx.apply(s.unwrap_or_default()), wx.apply(e.unwrap_or_default()))
            }
            None => (FVector::ZERO, FVector::ZERO),
        }};
        let additional = kick.then(|| (self.geo.character_socket_world(p, "AdditionalKickTracerStart"), self.geo.character_socket_world(p, "AdditionalKickTracerEnd")));
        if let Some((start, end)) = additional {
            let t = p.additional_tracer.get_or_insert_with(UeTracer::default);
            update_tracer(t, start, end);
        } else { p.additional_tracer = None; }
        let t = &mut p.tracer;
        update_tracer(t, cs, ce);
    }

    /// AMordhauWeapon::OnAttackStarted_Implementation rva=0x162eb40: ResetTracers (vcall +0x7d8), then
    /// RepositionClashCollider(Move > 1) (vcall +0x820, rva=0x163b150): in mesh space, with GetTrace's sockets S, E
    /// (Second* in alternate mode) and D = E - S: centre = S + D * 0.5 + ClashNormal * 10 * !bIsCentered
    /// - normalize(D) * 15, SetCapsuleSize(CapsuleRadius, |D| * 0.5 + 15); the relative rotation is kept (template:
    /// zero, so the capsule axis is the mesh Z). ClashNormal is SecondClashNormal in alternate mode (SwitchMode
    /// rva=0x1640a00 swaps them). Kick uses its character socket overrides and virtual weapon identity.
    fn reset(&self, w: &World, fi: usize) {
        let kick = self.is_kick(w, fi);
        let wg = self.attack_weapon(w, fi).cloned();
        let mv = w.cur_m(fi).and_then(|m| m.attack()).map(|a| a.mv).unwrap_or(0);
        let alt = w.fighters[fi].alternate_mode;
        if let Some(p) = self.posed.borrow_mut().get_mut(&w.fighters[fi].name) {
            p.tracer.invalidated = true;
            if kick { p.additional_tracer.get_or_insert_with(UeTracer::default).invalidated = true; }
            else { p.additional_tracer = None; }
            p.clash = wg.and_then(|g| {
                let (s, e) = if alt { (g.second_trace_start, g.second_trace_end) } else { (g.trace_start, g.trace_end) };
                let (s, e) = (s?, e?);
                let d = e - s;
                let l2 = d.dot(d);
                let dn = if l2 == 1.0 { d } else if l2 < 1e-8 { FVector::ZERO } else { d.scale(mordhau_core::ue::ue_inv_sqrt(l2) as f64) };
                let n = if alt { g.second_clash_normal } else { g.clash_normal };
                let off = if mv > 1 { 0.0 } else { 10.0 };
                let c = FVector::new(
                    (n.x * off + s.x + d.x * 0.5) - dn.x * 15.0,
                    (n.y * off + s.y + d.y * 0.5) - dn.y * 15.0,
                    (n.z * off + s.z + d.z * 0.5) - dn.z * 15.0,
                );
                Some((c, l2.sqrt() * 0.5 + 15.0, g.clash_radius))
            });
        }
    }

    fn sample_ex(&self, w: &World, fi: usize) -> TraceSample { self.collect(w, fi) }

    fn sample(&self, w: &World, fi: usize) -> (Vec<Vec<RawHit>>, FVector, FVector) {
        let sample = self.collect(w, fi);
        (sample.segments, sample.cur_start, sample.cur_end)
    }

    fn world_static_trace(&self, start: FVector, end: FVector) -> Option<FVector> {
        let query = self.world_static.borrow().clone()?;
        query(start, end).map(|hit| hit.impact_point)
    }
}

impl SimTrace {
    fn collect(&self, w: &World, fi: usize) -> TraceSample {
        let posed = self.posed.borrow();
        let Some(me) = posed.get(&w.fighters[fi].name) else { return TraceSample::default() };
        let t = me.tracer.clone();
        let mut out = Vec::new();
        if !(t.cur_valid && t.prev_valid) {
            return TraceSample { segments: out, cur_start: t.cur_start, cur_end: t.cur_end, blocking: None, last_dir: t.last_observed_direction };
        }
        let g = &self.geo;
        let mv = w.cur_m(fi).and_then(|m| m.attack()).map(|a| a.mv).unwrap_or(0);
        let extra = self.attack_weapon(w, fi).is_some_and(|weapon| weapon.extra_environment_tracers) && !self.is_kick(w, fi) && !matches!(mv, 4 | 5);
        let query = self.world_static.borrow().clone();
        let mut blocking = None;
        // every other character's body shapes in world space, once per call
        let mut bodies: Vec<(usize, &BodyShape, FTransform)> = Vec::new();
        for vi in 0..w.fighters.len() {
            if vi == fi {
                continue;
            }
            let Some(p) = posed.get(&w.fighters[vi].name) else { continue };
            for (k, s) in g.shapes.iter().enumerate() {
                let Some(b) = g.shape_bones[k] else { continue };
                if b >= p.bones.len() {
                    continue;
                }
                bodies.push((vi, s, s.xf.then(&g.bone_world(p, b))));
            }
        }
        let mut clashes: Vec<(usize, FTransform, crate::physics::Shape)> = Vec::new();
        for vi in 0..w.fighters.len() {
            if vi == fi || !w.fighters[vi].clash_collider_enabled {
                continue;
            }
            let (Some(p), Some(wg)) = (posed.get(&w.fighters[vi].name), self.weapon_of(w, vi)) else { continue };
            let (Some((c, hh, r)), Some(wx)) = (p.clash, g.weapon_world(p, wg)) else { continue };
            // UCapsuleComponent: CapsuleHalfHeight includes the radius (clamped to >= radius)
            let half_len = (hh.max(r) - r).max(0.0);
            clashes.push((vi, FTransform::new(wx.rot, wx.apply(c)), crate::physics::Shape::Capsule { radius: r, half_len }));
        }
        // shields' BlockColliders (always on: WeaponOnly profile, never toggled) on the fighters' left-hand equipment
        let mut shields: Vec<(usize, FTransform, crate::physics::Shape)> = Vec::new();
        for vi in 0..w.fighters.len() {
            let f = &w.fighters[vi];
            if vi == fi || f.left_hand_path.is_empty() {
                continue;
            }
            let (Some(p), Some(lg)) = (posed.get(&f.name), g.weapons.get(&f.left_hand_path)) else { continue };
            let (Some((rel, half)), Some(wx)) = (lg.block_box, g.weapon_world(p, lg)) else { continue };
            shields.push((vi, FTransform::new(wx.rot, wx.apply(rel)), crate::physics::Shape::Box { half }));
        }
        // SampleTracers 0x163c430: primary sweeps first, then additional without hand/environment extras.
        let mut passes = vec![(&t, extra)];
        if let Some(additional) = me.additional_tracer.as_ref().filter(|a| a.cur_valid && a.prev_valid) { passes.push((additional, false)); }
        for (tracer, extra) in passes {
        let dir_c = ue_safe_normal(tracer.cur_start - tracer.cur_end);
        let dir_p = ue_safe_normal(tracer.prev_start - tracer.prev_end);
        let len = (tracer.cur_end - tracer.cur_start).length();
        let n = cvtss2si((len * g.count_scale + g.round_bias) as f64) >> 1;
        let mut i = n as f32 + if extra { 4.0 } else { 0.0 };
        while i >= 0.0 {
            let o = i * g.spacing_cm;
            let a = tracer.prev_end + dir_p.scale(o as f64);
            let b = tracer.cur_end + dir_c.scale(o as f64);
            let hand_trace = i > n as f32;
            if blocking.is_none() {
                if let Some(hit) = query.as_ref().and_then(|query| query(a, b)) { blocking = Some((out.len(), hit)); }
            }
            // Combined local debug display: native authority blade=green, cosmetic hand/environment=blue.
            // Retain category identity; this does not add another cosmetic/gameplay trace pass or network RPC.
            self.sampled.borrow_mut().push(SampledTrace { fighter: fi, tick: w.tick_n as u64, start: a, end: b, environment_only: hand_trace });
            let mut hits = Vec::new();
            if !hand_trace {
            for (vi, s, x) in &bodies {
                if let Some(tt) = segment_shape(a, b, x, &s.shape) {
                    hits.push(RawHit { t: tt as f64, victim: *vi, comp: HitComp::Body(s.bone.clone()), trace_start: a, trace_end: b });
                }
            }
            // enabled BlockColliders (AMordhauCharacter EnableBlockCollider: QueryOnly while attacking / parrying)
            for vi in 0..w.fighters.len() {
                let f = &w.fighters[vi];
                if vi == fi || !f.block_collider_enabled {
                    continue;
                }
                if let Some(gm) = &f.geom {
                    let shape = crate::physics::Shape::Box { half: gm.block_extent };
                    if let Some(tt) = segment_shape(a, b, &gm.block_collider, &shape) {
                        hits.push(RawHit { t: tt as f64, victim: vi, comp: HitComp::BlockCollider, trace_start: a, trace_end: b });
                    }
                }
            }
            }
            // enabled weapon ClashColliders (OnAttackStarted enables, OnAttackStopped / UAttackMotion OnLeave disable):
            // the capsule under the weapon mesh; the hit's victim is the weapon's owner
            for (vi, x, shape) in &clashes {
                if let Some(tt) = segment_shape(a, b, x, shape) {
                    hits.push(RawHit { t: tt as f64, victim: *vi, comp: HitComp::Clash, trace_start: a, trace_end: b });
                }
            }
            if !hand_trace {
            for (vi, x, shape) in &shields {
                if let Some(tt) = segment_shape(a, b, x, shape) {
                    hits.push(RawHit { t: tt as f64, victim: *vi, comp: HitComp::ShieldBlock, trace_start: a, trace_end: b });
                }
            }
            }
            out.push(hits);
            i += g.step;
        }
        }
        TraceSample { segments: out, cur_start: t.cur_start, cur_end: t.cur_end, blocking, last_dir: t.last_observed_direction }
    }
}

fn update_tracer(t: &mut UeTracer, start: FVector, end: FVector) {
    t.prev_start = t.cur_start; t.prev_end = t.cur_end;
    t.cur_start = start; t.cur_end = end;
    t.prev_valid = t.cur_valid;
    t.cur_valid = !t.invalidated;
    t.invalidated = false;
    t.last_observed_direction = ue_safe_normal((t.cur_start - t.prev_start) + (t.cur_end - t.prev_end));
}

#[cfg(test)]
mod world_contact_tests {
    use super::*;
    use mordhau_core::data::{Spec, Stat, TracerDef, WeaponData, WeaponSetup};
    use crate::pose::Skeleton;

    fn fixture() -> (World, SimTrace) {
        let mut spec = Spec::default();
        // add_fighter starts native idle; the fixture must define that motion before querying traces.
        spec.motion_defs.insert("native:UIdleMotion".into(), Rc::new(mordhau_core::data::MotionDef::default()));
        spec.stats.insert("UHealthStatComponent".into(), Stat {initial_value:100,max_value:100,..Default::default()});
        spec.stats.insert("UStaminaStatComponent".into(), Stat {initial_value:100,max_value:100,..Default::default()});
        spec.weapons.insert("fixture".into(), WeaponSetup { weapon: Rc::new(WeaponData::default()), equip: None,
            motions: Default::default(), tracer: TracerDef::default() });
        let mut world = World::new(Rc::new(spec),1./60.);
        world.add_fighter("attacker","fixture",""); world.add_fighter("other","fixture","");
        world.fighters[1].clash_collider_enabled = true;
        let mut weapons = HashMap::new();
        weapons.insert("fixture".into(),WeaponGeo { path:"fixture".into(),right_handed:true,extra_environment_tracers:true,..Default::default() });
        let geo = Rc::new(Geometry {
            shapes:vec![BodyShape { bone:"body".into(),xf:FTransform::IDENTITY,shape:crate::physics::Shape::Box { half:FVector::new(1.,1.,1.) } }],
            shape_bones:vec![Some(0)],skeleton:Skeleton::from_parts(vec!["body".into(),"RightWeapon".into()],vec![-1,0],vec![FTransform::IDENTITY;2]),
            mesh_xf:FTransform::IDENTITY,character_sockets:HashMap::new(),block_collider_rel:FVector::ZERO,block_collider_extent:FVector::ZERO,
            block_collider_offsets:[mordhau_core::combat::geometry::ScaledXf {rot:mordhau_core::ue::FQuat::IDENTITY,loc:FVector::ZERO,scale:FVector::new(1.,1.,1.)};3],weapons,grip_pitch_right:0.,grip_pitch_left:0.,
            count_scale:0.26666668,round_bias:0.5,spacing_cm:7.5,step:-1.,
        });
        let mut posed = HashMap::new();
        posed.insert("attacker".into(),Posed { bones:vec![FTransform::IDENTITY;2],tracer:UeTracer {
            prev_start:FVector::new(15.,0.,0.),prev_end:FVector::ZERO,
            cur_start:FVector::new(15.,10.,0.),cur_end:FVector::new(0.,10.,0.),cur_valid:true,prev_valid:true,..Default::default()
        },..Default::default() });
        // The first extra segment x45 passes through BOTH a body and a weapon clash. Native hand filtering retains only clash.
        posed.insert("other".into(),Posed { actor:FTransform::new(mordhau_core::ue::FQuat::IDENTITY,FVector::new(45.,5.,0.)),
            bones:vec![FTransform::IDENTITY;2],clash:Some((FVector::ZERO,1.,1.)),..Default::default() });
        let trace = SimTrace { geo,posed:Rc::new(RefCell::new(posed)),sampled:Rc::new(RefCell::new(Vec::new())),world_static:Rc::new(RefCell::new(None)) };
        (world,trace)
    }

    #[test]
    fn native_first_segment_latch_and_hand_filter_do_not_choose_global_nearest() {
        let (world,trace) = fixture();
        let calls = Rc::new(RefCell::new(Vec::new())); let seen=calls.clone();
        *trace.world_static.borrow_mut()=Some(Rc::new(move |a,b| {
            seen.borrow_mut().push((a,b));
            Some(WorldBlock { impact_point:a,actor:true,..Default::default() })
        }));
        let sample=trace.sample_ex(&world,0);
        assert_eq!(sample.segments.len(),7); // n2 + four preceding extras + endpoint0.
        assert_eq!(calls.borrow().len(),1,"Native world query stops after the first segment hit");
        // Original rsqrtss + two binary32 refinements is not an exact real-number unit vector.
        // Geometric oracle allowance is <1e-5cm; draw-vs-query identity is still exact below.
        assert!((calls.borrow()[0].0-FVector::new(45.,0.,0.)).length()<1e-5);
        assert_eq!(sample.blocking.unwrap().0,0);
        assert!(sample.segments[0].iter().any(|h|h.comp==HitComp::Clash),"Hand trace must still accept weapon clash");
        assert!(sample.segments[..4].iter().flatten().all(|h|h.comp==HitComp::Clash),"Hand traces must reject body/block/shield hits");
        let published = trace.sampled.borrow();
        assert_eq!(published.len(),7,"Combined local display includes the four existing environment sweeps");
        assert_eq!(published.iter().filter(|t|t.environment_only).count(),4);
        assert_eq!(published.iter().filter(|t|!t.environment_only).count(),3,"Original ClientDrawTracer category excludes hand sweeps");
        for (i,t) in published.iter().enumerate() {
            assert_eq!(t.environment_only,i<4);
            assert!((t.start-FVector::new(45.-i as f32*7.5,0.,0.)).length()<1e-5);
            assert!((t.end-FVector::new(45.-i as f32*7.5,10.,0.)).length()<1e-5);
        }
        assert_eq!((published[0].start,published[0].end),calls.borrow()[0]);
        drop(published);
        // The original presentation retrace reaches the same provider rather than an independent movement hull.
        let point=FVector::new(1.,2.,3.);
        assert_eq!(trace.world_static_trace(point,FVector::ZERO),Some(point));
        assert_eq!(calls.borrow().len(),2);
    }

    #[test]
    fn absent_provider_and_invalid_previous_tracers_preserve_no_world_contact() {
        let (world,trace)=fixture();
        assert!(trace.sample_ex(&world,0).blocking.is_none());
        trace.sampled.borrow_mut().clear();
        trace.posed.borrow_mut().get_mut("attacker").unwrap().tracer.prev_valid=false;
        let calls=Rc::new(RefCell::new(0));let seen=calls.clone();
        *trace.world_static.borrow_mut()=Some(Rc::new(move |_,_| { *seen.borrow_mut()+=1;None }));
        let sample=trace.sample_ex(&world,0);
        assert!(sample.segments.is_empty() && sample.blocking.is_none());
        assert_eq!(*calls.borrow(),0);assert!(trace.sampled.borrow().is_empty());
    }

    #[test]
    fn additional_sweep_reaches_body_without_environment_filter() {
        let (world, trace) = fixture();
        trace.posed.borrow_mut().get_mut("attacker").unwrap().additional_tracer = Some(UeTracer {
            prev_start:FVector::new(45.,0.,0.),prev_end:FVector::new(45.,0.,0.),
            cur_start:FVector::new(45.,10.,0.),cur_end:FVector::new(45.,10.,0.),cur_valid:true,prev_valid:true,..Default::default()
        });
        let sample = trace.sample_ex(&world, 0);
        assert_eq!(sample.segments.len(), 8);
        assert!(sample.segments[7].iter().any(|h| matches!(h.comp, HitComp::Body(_))));
        assert!(!trace.sampled.borrow()[7].environment_only);
    }

    #[test]
    fn character_socket_uses_animated_bone_not_held_weapon() {
        let (_, trace) = fixture();
        let mut geo = Rc::try_unwrap(trace.geo).ok().unwrap();
        geo.character_sockets.insert("KickTracerEnd".into(), (0, FTransform::new(mordhau_core::ue::FQuat::IDENTITY,FVector::new(2.,3.,4.))));
        let mut p = Posed { actor:FTransform::new(mordhau_core::ue::FQuat::IDENTITY,FVector::new(100.,0.,0.)),bones:vec![FTransform::new(mordhau_core::ue::FQuat::IDENTITY,FVector::new(10.,0.,0.))],..Default::default() };
        assert_eq!(geo.character_socket_world(&p,"KickTracerEnd"),FVector::new(112.,3.,4.));
        p.bones[0].loc.x = 20.;
        assert_eq!(geo.character_socket_world(&p,"KickTracerEnd"),FVector::new(122.,3.,4.));
    }
}

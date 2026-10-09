//! pak_fighter.rs - a fighter's skinned body and an animation clip decoded from the paks with mh-assets
//! (`--assets pak`): the Rust side of the Godot `pak` backend's UeAnim (godot/components/ue/ue_anim.gd: UeSkeletalMesh
//! -> Skeleton3D + skinned mesh, UeAnimSequence.to_animation with the Skeleton-mode retarget).
//!
//! Sources: mesh = UMA_Master (ue_anim.gd CLIP_MESH, the mesh every exported clip was written onto); clip = the R1 idle.
//! Tracks index the clip's USkeleton bones (anim.rs Track.bone); they are matched to the mesh's bones by name
//! (ue_anim.gd header: mesh and clips share UMA_Master_Skeleton). Translation: bones whose BoneTree
//! TranslationRetargetingMode is Skeleton keep the mesh's reference translation, others take the clip's
//! (ue_anim_sequence.gd to_animation; FAnimationRuntime::RetargetBoneTransform via CUE4Parse CAnimSequence.cs:111-160).
//! Sampling: the exe's sampler (AnimSequence::sample: FastLerp rotation, linear translation; AEFPerTrackCompressionCodec
//! 0x142e6b330) at SAMPLE_HZ, then Bevy interpolates between samples (slerp) - a sub-sample approximation.

use bevy::animation::animation_curves::{AnimatableCurve, AnimatedField};
use bevy::animation::{animated_field, AnimationClip, AnimationTargetId};
use bevy::asset::RenderAssetUsages;
use bevy::math::curve::UnevenSampleAutoCurve;
use bevy::mesh::{Indices, Mesh, PrimitiveTopology, VertexAttributeValues};
use bevy::prelude::*;
use mh_assets::coords;
use mh_assets::pak_source::PakSource;
use mh_assets::skeletal_mesh::{self, SkeletalMesh};

pub const BODY_MESH: &str = "Mordhau/Content/UMA/UMA/Master/UMA_Master";
pub const IDLE_ANIM: &str = "Mordhau/Content/Mordhau/Animations/RawClips/1H/1H_RH_Idle_3p_var3_SwordNew";
pub const SAMPLE_HZ: f32 = 60.0;

pub struct Bone {
    pub name: String,
    pub parent: i32,
    pub local: Transform,
}

pub struct PakFighter {
    pub bones: Vec<Bone>,
    /// per bone: inverse of the bind (reference) pose in mesh space
    pub inverse_bindposes: Vec<Mat4>,
    /// (mesh, material package)
    pub parts: Vec<(Mesh, String)>,
    pub clip: AnimationClip,
    pub clip_tracks: usize,
    pub clip_tracks_unmatched: usize,
    pub vertices: usize,
}

pub fn ue_local(t: &skeletal_mesh::Transform) -> Transform {
    let q = coords::quat(t.rotation);
    Transform { translation: Vec3::from(coords::pos(t.translation)), rotation: Quat::from_xyzw(q[0], q[1], q[2], q[3]), scale: Vec3::from(coords::scale(t.scale)) }
}

/// Path names from the top bone down to `i` (the AnimationTargetId of that joint)
pub fn bone_path(bones: &[Bone], i: usize) -> Vec<Name> {
    let mut v = Vec::new();
    let mut c = i as i32;
    while c >= 0 {
        v.push(Name::new(bones[c as usize].name.clone()));
        c = bones[c as usize].parent;
    }
    v.reverse();
    v
}

pub fn target_id(bones: &[Bone], i: usize) -> AnimationTargetId {
    AnimationTargetId::from_names(bone_path(bones, i).iter())
}

/// LOD0 sections of a skeletal mesh as Bevy meshes (skinned attributes included) + slot material package
pub fn mesh_parts(sm: &SkeletalMesh) -> Vec<(Mesh, String)> {
    parts(sm)
}

fn parts(sm: &SkeletalMesh) -> Vec<(Mesh, String)> {
    let v = &sm.vertices;
    let mut out = Vec::new();
    for s in &sm.sections {
        if s.num_triangles <= 0 || s.num_vertices <= 0 {
            continue;
        }
        let lo = s.base_vertex_index as usize;
        let hi = (lo + s.num_vertices as usize).min(v.positions.len());
        let r = lo..hi;
        let mut m = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
        m.insert_attribute(Mesh::ATTRIBUTE_POSITION, v.positions[r.clone()].iter().map(|p| coords::pos(*p)).collect::<Vec<_>>());
        if v.normals.len() == v.positions.len() {
            m.insert_attribute(Mesh::ATTRIBUTE_NORMAL, v.normals[r.clone()].iter().map(|n| coords::dir(*n)).collect::<Vec<_>>());
        }
        if let Some(uv) = v.uvs.first().filter(|u| u.len() == v.positions.len()) {
            m.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv[r.clone()].to_vec());
        }
        // 4 strongest influences, renormalized (Bevy skins with 4; UE stores 4 or 8)
        let k = sm.max_influences;
        let (mut ji, mut jw) = (Vec::with_capacity(r.len()), Vec::with_capacity(r.len()));
        for vi in r.clone() {
            let w = sm.normalized_weights(vi);
            let mut ix: Vec<usize> = (0..k).collect();
            ix.sort_by(|a, b| w[*b].partial_cmp(&w[*a]).unwrap_or(std::cmp::Ordering::Equal));
            let top: Vec<usize> = ix.into_iter().take(4).collect();
            let s: f32 = top.iter().map(|&i| w[i]).sum::<f32>().max(1e-6);
            ji.push([0, 1, 2, 3].map(|n| sm.bones[vi * k + top[n]]));
            jw.push([0, 1, 2, 3].map(|n| w[top[n]] / s));
        }
        m.insert_attribute(Mesh::ATTRIBUTE_JOINT_INDEX, VertexAttributeValues::Uint16x4(ji));
        m.insert_attribute(Mesh::ATTRIBUTE_JOINT_WEIGHT, jw);
        let a = s.base_index.max(0) as usize;
        let b = (a + 3 * s.num_triangles as usize).min(sm.indices.len());
        m.insert_indices(Indices::U32(sm.indices[a..b].iter().map(|i| i.saturating_sub(lo as u32)).collect()));
        let mat = sm.materials.get(s.material_index.max(0) as usize).map(|m| m.material_package.clone()).unwrap_or_default();
        out.push((m, mat));
    }
    out
}

pub fn build(src: &PakSource, rd: &mh_pak::Reader, mesh_pkg: &str, anim_pkg: &str, include_art: bool) -> Result<PakFighter, String> {
    let sm = if include_art { Some(skeletal_mesh::lod0(src, mesh_pkg).map_err(|e| e.0)?) } else { None };
    let reference = match &sm {
        Some(sm) => sm.ref_skeleton.clone(),
        None => skeletal_mesh::mesh_reference(src, mesh_pkg).map_err(|e| e.0)?,
    };
    let bones: Vec<Bone> = reference
        .bones
        .iter()
        .zip(reference.pose.iter())
        .map(|(b, t)| Bone { name: b.name.clone(), parent: b.parent, local: ue_local(t) })
        .collect();
    // bind pose: global = parent global * local, in Y-up space
    let mut global = vec![Mat4::IDENTITY; bones.len()];
    for i in 0..bones.len() {
        let l = bones[i].local.to_matrix();
        global[i] = if bones[i].parent >= 0 { global[bones[i].parent as usize] * l } else { l };
    }
    let inverse_bindposes = global.iter().map(|g| g.inverse()).collect();
    // clip
    let seq = mh_assets::anim::decode(src, anim_pkg).map_err(|e| e.0)?;
    let skel_pkg = rd
        .read(anim_pkg)
        .and_then(|ex| ex.iter().find_map(|e| e.pointer("/Properties/Skeleton/ObjectPath").and_then(|s| s.as_str()).map(|s| mh_pak::reader::strip_index(s).to_string())))
        .ok_or(format!("{anim_pkg}: no Skeleton reference"))?;
    let (skel, modes) = skeletal_mesh::skeleton(src, &skel_pkg).map_err(|e| e.0)?;
    let mut clip = AnimationClip::default();
    let n = ((seq.sequence_length * SAMPLE_HZ).ceil() as usize).max(1) + 1;
    let times: Vec<f32> = (0..n).map(|i| seq.sequence_length * i as f32 / (n - 1) as f32).collect();
    let (mut matched, mut unmatched) = (0, 0);
    for t in &seq.tracks {
        let Some(bname) = skel.bones.get(t.bone.max(0) as usize).map(|b| b.name.clone()) else {
            unmatched += 1;
            continue;
        };
        let Some(mi) = reference.find(&bname) else {
            unmatched += 1;
            continue;
        };
        matched += 1;
        let id = target_id(&bones, mi);
        let skeleton_mode = modes.get(t.bone.max(0) as usize).is_some_and(|m| m.ends_with("::Skeleton"));
        let (mut rots, mut poss, mut scls) = (Vec::new(), Vec::new(), Vec::new());
        for &tm in &times {
            let (r, p, s) = seq.sample(t, tm);
            if let Some(r) = r {
                let q = coords::quat(r);
                rots.push((tm, Quat::from_xyzw(q[0], q[1], q[2], q[3])));
            }
            if let Some(p) = p.filter(|_| !skeleton_mode) {
                poss.push((tm, Vec3::from(coords::pos(p))));
            }
            if let Some(s) = s {
                scls.push((tm, Vec3::from(coords::scale(s))));
            }
        }
        if rots.len() >= 2 {
            if let Ok(c) = UnevenSampleAutoCurve::new(rots) {
                clip.add_curve_to_target(id, AnimatableCurve::new(animated_field!(Transform::rotation), c));
            }
        }
        if poss.len() >= 2 {
            if let Ok(c) = UnevenSampleAutoCurve::new(poss) {
                clip.add_curve_to_target(id, AnimatableCurve::new(animated_field!(Transform::translation), c));
            }
        }
        if scls.len() >= 2 {
            if let Ok(c) = UnevenSampleAutoCurve::new(scls) {
                clip.add_curve_to_target(id, AnimatableCurve::new(animated_field!(Transform::scale), c));
            }
        }
    }
    let (parts, vertices) = sm.as_ref().map(|sm| (parts(sm), sm.vertices.positions.len())).unwrap_or_default();
    Ok(PakFighter { inverse_bindposes, parts, vertices, bones, clip, clip_tracks: matched, clip_tracks_unmatched: unmatched })
}

/// One skeletal mesh part bound by bone name to the master skeleton (character_builder.gd: every part shares
/// UMA_Master_Skeleton, its skin rebound by name). `joints` = the part's reference-skeleton bone names; the inverse
/// bind poses come from the part's own reference pose.
#[allow(dead_code)] // vertices: evidence
pub struct SkinPart {
    pub meshes: Vec<(Mesh, String)>,
    pub joints: Vec<String>,
    pub inverse_bindposes: Vec<Mat4>,
    pub vertices: usize,
}

pub fn skin_part(src: &PakSource, pkg: &str) -> Result<SkinPart, String> {
    let sm = skeletal_mesh::lod0(src, pkg).map_err(|e| e.0)?;
    let rs = &sm.ref_skeleton;
    let mut global = vec![Mat4::IDENTITY; rs.bones.len()];
    for i in 0..rs.bones.len() {
        let l = ue_local(&rs.pose[i]).to_matrix();
        let p = rs.bones[i].parent;
        global[i] = if p >= 0 { global[p as usize] * l } else { l };
    }
    Ok(SkinPart {
        meshes: parts(&sm),
        joints: rs.bones.iter().map(|b| b.name.clone()).collect(),
        inverse_bindposes: global.iter().map(|g| g.inverse()).collect(),
        vertices: sm.vertices.positions.len(),
    })
}

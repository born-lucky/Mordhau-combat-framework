//! Character poses in UE space: the reference skeleton (mh-assets FReferenceSkeleton of the mesh's USkeleton),
//! component-space bone transforms from a reference pose or an AnimSequence sample (mh-assets anim, the exe's
//! sampler), the actor -> mesh placement (BP_MordhauCharacter CharacterMesh0 RelativeLocation / RelativeRotation), and
//! the held weapon's actor transform on its hand socket (UEquipmentSystemComponent::ComputeGrippedTransform
//! rva=0x14b70f0 as godot/game/actor/held_weapon.gd ports it).

use mh_assets::anim::AnimSequence;
use mh_assets::skeletal_mesh::RefSkeleton;
use mordhau_core::ue::{FQuat, FTransform, FVector};
use std::collections::HashMap;

#[derive(Clone, Debug)]
pub struct Skeleton {
    pub names: Vec<String>,
    pub parents: Vec<i32>,
    /// local (parent-relative) reference pose
    pub ref_local: Vec<FTransform>,
    translation_modes: Vec<TranslationMode>,
    index: HashMap<String, usize>,
}

/// Original EBoneTranslationRetargetingMode values supported by the shipped UMA character BoneTree.
/// Other modes require source retarget poses/scale and must not silently become Animation.
#[derive(Clone, Copy, Debug, PartialEq)]
enum TranslationMode {
    Animation,
    Skeleton,
}

impl Skeleton {
    pub fn from_ref(r: &RefSkeleton) -> Skeleton {
        let names: Vec<String> = r.bones.iter().map(|b| b.name.clone()).collect();
        let index = names.iter().enumerate().map(|(i, n)| (n.to_lowercase(), i)).collect();
        Skeleton {
            parents: r.bones.iter().map(|b| b.parent).collect(),
            ref_local: r.pose.iter().map(|t| FTransform::new(FQuat::from_array(t.rotation), FVector::new(t.translation[0], t.translation[1], t.translation[2]))).collect(),
            names,
            index,
            translation_modes: vec![TranslationMode::Animation; r.bones.len()],
        }
    }

    /// Bind the original USkeleton BoneTree to this target mesh's reference pose by FName and parent identity.
    /// The output remains in USkeleton track-index order. Virtual bones are not guessed from a mesh index.
    pub fn from_retargeted_ref(source: &RefSkeleton, modes: &[String], target: &RefSkeleton) -> Result<Skeleton, String> {
        if modes.len() != source.bones.len() || source.pose.len() != source.bones.len() || target.pose.len() != target.bones.len() {
            return Err("reference skeleton / BoneTree count mismatch".into());
        }
        let target_index: HashMap<String, usize> = target.bones.iter().enumerate().map(|(i, b)| (b.name.to_lowercase(), i)).collect();
        if target_index.len() != target.bones.len() {
            return Err("target reference skeleton has duplicate bone names".into());
        }
        let mut sk = Self::from_ref(source);
        if sk.index.len() != source.bones.len() {
            return Err("source reference skeleton has duplicate bone names".into());
        }
        for (i, b) in source.bones.iter().enumerate() {
            sk.translation_modes[i] = match modes[i].rsplit("::").next().unwrap_or("") {
                "" | "Animation" => TranslationMode::Animation,
                "Skeleton" => TranslationMode::Skeleton,
                other => return Err(format!("{}: unsupported translation retargeting mode {other}", b.name)),
            };
            let j = *target_index.get(&b.name.to_lowercase()).ok_or_else(|| format!("target reference missing bone {}", b.name))?;
            let parent_name = |r: &RefSkeleton, p: i32| -> Result<Option<String>, String> {
                if p == -1 {
                    Ok(None)
                } else {
                    r.bones.get(p as usize).map(|b| Some(b.name.to_lowercase())).ok_or_else(|| "invalid reference parent index".into())
                }
            };
            if parent_name(source, b.parent)? != parent_name(target, target.bones[j].parent)? {
                return Err(format!("{}: source / target reference parent mismatch", b.name));
            }
            let t = &target.pose[j];
            sk.ref_local[i] = FTransform::new(FQuat::from_array(t.rotation), FVector::new(t.translation[0], t.translation[1], t.translation[2]));
        }
        Ok(sk)
    }
    /// a skeleton from its parts (tests and synthetic rigs)
    pub fn from_parts(names: Vec<String>, parents: Vec<i32>, ref_local: Vec<FTransform>) -> Skeleton {
        let index = names.iter().enumerate().map(|(i, n)| (n.to_lowercase(), i)).collect();
        let translation_modes = vec![TranslationMode::Animation; names.len()];
        Skeleton { names, parents, ref_local, translation_modes, index }
    }
    /// bone index by name, ignoring case (FName)
    pub fn find(&self, name: &str) -> Option<usize> {
        self.index.get(&name.to_lowercase()).copied()
    }
    /// component space from local transforms (parents precede children in an FReferenceSkeleton)
    pub fn to_component(&self, local: &[FTransform]) -> Vec<FTransform> {
        let mut out: Vec<FTransform> = Vec::with_capacity(local.len());
        for (i, l) in local.iter().enumerate() {
            let p = self.parents[i];
            out.push(if p < 0 { *l } else { l.then(&out[p as usize]) });
        }
        out
    }
    /// component -> local (parent-relative) pose
    pub fn to_local(&self, cs: &[FTransform]) -> Vec<FTransform> {
        (0..cs.len())
            .map(|i| {
                let p = self.parents[i];
                if p >= 0 {
                    let pi = cs[p as usize].inverse();
                    FTransform::new(pi.rot.mul(cs[i].rot), pi.apply(cs[i].loc))
                } else {
                    cs[i]
                }
            })
            .collect()
    }

    pub fn ref_pose(&self) -> Vec<FTransform> {
        self.to_component(&self.ref_local)
    }
    /// An AnimSequence at time t on this skeleton (tracks map to reference-skeleton bone indices; untracked bones and
    /// absent channels keep the target reference pose; scale ignored). Original BoneTree modes apply to translation.
    pub fn sample(&self, anim: &AnimSequence, t: f32) -> Vec<FTransform> {
        self.to_component(&self.sample_local(anim, t))
    }
    /// the same in local (parent-relative) space
    pub fn sample_local(&self, anim: &AnimSequence, t: f32) -> Vec<FTransform> {
        let mut local = self.ref_local.clone();
        for tr in &anim.tracks {
            let b = tr.bone as usize;
            if b >= local.len() {
                continue;
            }
            let (r, p, _s) = anim.sample(tr, t);
            if let Some(r) = r {
                local[b].rot = FQuat::from_array(r);
            }
            if let Some(p) = p {
                local[b].loc = self.retarget_translation(b, FVector::new(p[0], p[1], p[2]), false);
            }
        }
        local
    }

    /// FAnimationRuntime::RetargetBoneTransform RVA 0x2e5a730, mode 1 branch 0x142e5ad4e:
    /// normal -> FBoneContainer target reference translation; baked additive -> zero. Mode 0 preserves animation.
    /// DecompressPose RVA 0x2e64b20 omits mode-1 translation decompression, leaving the same initialized values.
    pub fn retarget_translation(&self, bone: usize, sampled: FVector, baked_additive: bool) -> FVector {
        match self.translation_modes[bone] {
            TranslationMode::Animation => sampled,
            TranslationMode::Skeleton if baked_additive => FVector::ZERO,
            TranslationMode::Skeleton => self.ref_local[bone].loc,
        }
    }
}

/// World transform of a character's actor (capsule centre, yaw only: ACharacter keeps pitch / roll at 0)
pub fn actor_xf(location: FVector, yaw: f32) -> FTransform {
    FTransform::new(FQuat::from_rotator(0.0, yaw, 0.0), location)
}

/// The held weapon's actor transform: Socket * T(RightHandEquipOffset) * R(RotationOffset) * R(Pitch +-90) *
/// T(-GripLocationLocal) (held_weapon.gd header; ComputeGrippedTransform rva=0x14b70f0, quaternion order
/// Q = Qsocket * Qrot * Qpitch). `pitch` = PlayerConstants grip_pitch_right (.rdata 0x1440701fc) / _left.
pub fn gripped(socket_world: &FTransform, offset: FVector, rot_pyr: [f32; 3], pitch: f32, grip: FVector) -> FTransform {
    let t_grip = FTransform::new(FQuat::IDENTITY, FVector::new(-grip.x, -grip.y, -grip.z));
    let r_pitch = FTransform::new(FQuat::from_rotator(pitch, 0.0, 0.0), FVector::ZERO);
    let r_rot = FTransform::new(FQuat::from_rotator(rot_pyr[0], rot_pyr[1], rot_pyr[2]), FVector::ZERO);
    let t_off = FTransform::new(FQuat::IDENTITY, offset);
    t_grip.then(&r_pitch).then(&r_rot).then(&t_off).then(socket_world)
}

#[cfg(test)]
mod retarget_tests {
    use super::*;
    use mh_assets::anim::{Channel, Track};
    use mh_assets::skeletal_mesh::{BoneInfo, Transform};

    fn reference(names: &[(&str, i32, f32)]) -> RefSkeleton {
        RefSkeleton {
            bones: names.iter().map(|(name, parent, _)| BoneInfo { name: name.to_string(), parent: *parent }).collect(),
            pose: names.iter().map(|(_, _, x)| Transform { rotation: [0.0, 0.0, 0.0, 1.0], translation: [*x, 0.0, 0.0], scale: [1.0; 3] }).collect(),
        }
    }

    fn modes() -> Vec<String> {
        ["Animation", "Skeleton", "Animation"].map(|s| format!("EBoneTranslationRetargetingMode::{s}")).to_vec()
    }

    #[test]
    fn retarget_normal_and_baked_additive_use_original_distinct_translation_rules() {
        let source = reference(&[("Global", -1, 0.0), ("Hand", 0, 2.0), ("Weapon", 1, 3.0)]);
        let target = reference(&[("global", -1, 0.0), ("hand", 0, 5.0), ("weapon", 1, 7.0)]);
        let sk = Skeleton::from_retargeted_ref(&source, &modes(), &target).unwrap();
        let q = FQuat::from_rotator(10.0, 20.0, 30.0);
        let anim = AnimSequence {
            package: "synthetic".into(), num_frames: 1, sequence_length: 1.0, rate_scale: 1.0,
            additive_anim_type: "AAT_None".into(), step: false, codec: String::new(), properties: Default::default(), curves: Vec::new(),
            tracks: vec![
                Track { bone: 1, pos: Some(Channel { keys: vec![[90.0, 0.0, 0.0]], frames: Vec::new() }), rot: Some(Channel { keys: vec![[q.x, q.y, q.z, q.w]], frames: Vec::new() }), scale: None },
                Track { bone: 2, pos: Some(Channel { keys: vec![[80.0, 0.0, 0.0]], frames: Vec::new() }), rot: None, scale: None },
            ],
        };
        let pose = sk.sample_local(&anim, 0.0);
        assert_eq!(pose[1].loc, FVector::new(5.0, 0.0, 0.0)); // target mesh, not source Skeleton's 2
        assert_eq!(pose[1].rot, FQuat::from_array(anim.sample(&anim.tracks[0], 0.0).0.unwrap())); // rotation remains animated
        assert_eq!(pose[2].loc, FVector::new(80.0, 0.0, 0.0)); // Animation is not forced to reference
        assert_eq!(sk.retarget_translation(1, FVector::new(90.0, 0.0, 0.0), true), FVector::ZERO);
        assert_eq!(sk.retarget_translation(2, FVector::new(80.0, 0.0, 0.0), true), FVector::new(80.0, 0.0, 0.0));
    }

    #[test]
    fn retarget_rejects_unmapped_bones_modes_and_parent_identity() {
        let source = reference(&[("Global", -1, 0.0), ("Hand", 0, 2.0), ("Weapon", 1, 3.0)]);
        assert!(Skeleton::from_retargeted_ref(&source, &modes()[..2], &source).is_err());
        let mut unsupported = modes();
        unsupported[1] = "EBoneTranslationRetargetingMode::AnimationScaled".into();
        assert!(Skeleton::from_retargeted_ref(&source, &unsupported, &source).is_err());
        assert!(Skeleton::from_retargeted_ref(&source, &modes(), &reference(&[("Global", -1, 0.0), ("Other", 0, 2.0), ("Weapon", 1, 3.0)])).is_err());
        assert!(Skeleton::from_retargeted_ref(&source, &modes(), &reference(&[("Global", -1, 0.0), ("Hand", 0, 2.0), ("Weapon", 0, 3.0)])).is_err());
    }

    #[test]
    fn retarget_original_character_bone_tree_and_compressed_stab() {
        let vfs = match mh_pak::Vfs::mount_default() {
            Ok(vfs) => std::sync::Arc::new(vfs),
            Err(e) if std::env::var("MORDHAU_GOLDEN_REQUIRED").ok().as_deref() == Some("1") => panic!("original asset required: {e}"),
            Err(_) => return,
        };
        let src = mh_assets::pak_source::PakSource::new(vfs.clone());
        let (source, modes) = mh_assets::skeletal_mesh::skeleton(&src, "Mordhau/Content/UMA/UMA/Master/UMA_Master_Skeleton").unwrap();
        let target = mh_assets::skeletal_mesh::mesh_reference(&src, "Mordhau/Content/UMA/UMA/Master/UMA_Master").unwrap();
        let sk = Skeleton::from_retargeted_ref(&source, &modes, &target).unwrap();
        assert_eq!(sk.names.len(), 158);
        assert_eq!(sk.translation_modes.iter().filter(|m| **m == TranslationMode::Skeleton).count(), 29);
        assert_eq!(sk.translation_modes.iter().filter(|m| **m == TranslationMode::Animation).count(), 129);
        let c = mh_assets::anim::decode(&src, "Mordhau/Content/Mordhau/Animations/RawClips/2H/Sword/2H_Sword_RightStab").unwrap();
        let hand = sk.find("RightHand").unwrap();
        let target_hand = target.bones.iter().position(|b| b.name.eq_ignore_ascii_case("RightHand")).unwrap();
        assert_eq!(sk.translation_modes[hand], TranslationMode::Skeleton);
        for t in [0.0, 0.5, 1.0] {
            let pose = sk.sample_local(&c, t);
            let p = target.pose[target_hand].translation;
            assert_eq!(pose[hand].loc, FVector::new(p[0], p[1], p[2]));
            let tr = c.tracks.iter().find(|tr| tr.bone == hand as i32).unwrap();
            let (q, _, _) = c.sample(tr, t);
            assert_eq!(pose[hand].rot, FQuat::from_array(q.unwrap()));
        }
        let a = crate::animgraph::AnimAssets::new(vfs).unwrap();
        let add = crate::additive::additive_sample(&a, &sk, "Mordhau/Content/Mordhau/Animations/RawClips/Misc/Parry_Additive", 0.2);
        for (i, mode) in sk.translation_modes.iter().enumerate() {
            if *mode == TranslationMode::Skeleton {
                assert_eq!(add[i].map(|(_, p)| p).unwrap_or(FVector::ZERO), FVector::ZERO);
            }
        }
    }
}

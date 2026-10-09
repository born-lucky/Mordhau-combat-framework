//! Loading a Sim's data from the user's install: the spec matrix (data_gen/spec) for records, the paks for geometry.

use crate::physics::{body_shapes, character_mesh, strip, weapon_sockets_ue};
use crate::pose::Skeleton;
use crate::spec::SpecBuilder;
use crate::trace::{Geometry, WeaponGeo};
use mh_assets::pak_source::PakSource;
use mh_pak::Reader;
use mordhau_core::data::Spec;
use mordhau_core::ue::FVector;
use std::collections::HashMap;
use std::sync::Arc;

pub struct Loaded {
    pub spec: Spec,
    pub geo: Geometry,
}

fn v3(v: &serde_json::Value) -> FVector {
    let g = |i: usize| v.get(i).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
    FVector::new(g(0), g(1), g(2))
}

/// AMordhauEquipment bUseEquippedOffset / EquippedOffset (class defaults, the Blueprint chain merged): Some(offset)
/// when the flag is set (BP_MordhauShield: Rotation (0.1498, 0.5733, 0.0036, 0.8055), Translation (-14.69, 3.2, 1.17))
pub fn equipped_offset(rd: &Reader, bp: &str) -> Option<mordhau_core::ue::FTransform> {
    let d = rd.defaults(bp);
    if d.get("bUseEquippedOffset").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }
    Some(equipped_offset_raw(&d))
}

/// AMordhauEquipment EquippedOffset as stored (identity when absent), whatever b(Second)UseEquippedOffset says
pub fn equipped_offset_raw(d: &serde_json::Map<String, serde_json::Value>) -> mordhau_core::ue::FTransform {
    let eo = d.get("EquippedOffset");
    let f = |a: &str, b: &str, dflt: f64| eo.and_then(|o| o.get(a)).and_then(|o| o.get(b)).and_then(|v| v.as_f64()).unwrap_or(dflt) as f32;
    let q = mordhau_core::ue::FQuat::from_array([f("Rotation", "X", 0.0), f("Rotation", "Y", 0.0), f("Rotation", "Z", 0.0), f("Rotation", "W", 1.0)]);
    mordhau_core::ue::FTransform::new(q, FVector::new(f("Translation", "X", 0.0), f("Translation", "Y", 0.0), f("Translation", "Z", 0.0)))
}

/// Records (exe f32) + geometry for these weapons (left hands included in `weapons`)
pub fn load(m: &mh_spec::Spec, vfs: Arc<mh_pak::Vfs>, weapons: &[&str]) -> Result<Loaded, String> {
    let rd = Reader::new(vfs.clone());
    let spec = SpecBuilder::new(m, &rd).build(weapons)?;
    let (pa, mesh, mesh_xf) = character_mesh(&rd)?;
    let shapes = body_shapes(&rd, &pa)?;
    // the mesh's USkeleton (SkeletalMesh.Skeleton)
    let ex = rd.read(&mesh).ok_or_else(|| format!("pak: no mesh {mesh}"))?;
    let skel_path = ex
        .iter()
        .find(|e| e["Type"].as_str() == Some("SkeletalMesh"))
        .and_then(|e| e["Properties"]["Skeleton"]["ObjectPath"].as_str())
        .map(strip)
        .ok_or_else(|| format!("{mesh}: no Skeleton"))?;
    let src = PakSource::new(vfs);
    let (rs, modes) = mh_assets::skeletal_mesh::skeleton(&src, &skel_path).map_err(|e| e.0)?;
    let target_ref = mh_assets::skeletal_mesh::mesh_reference(&src, &mesh).map_err(|e| e.0)?;
    let skeleton = Skeleton::from_retargeted_ref(&rs, &modes, &target_ref)?;
    let shape_bones = shapes.iter().map(|s| skeleton.find(&s.bone)).collect();
    let mut wg = HashMap::new();
    let mut paths: Vec<String> = weapons.iter().map(|s| s.to_string()).collect();
    paths.push(spec.kick_weapon_path.clone());
    for p in paths.iter().filter(|p| !p.is_empty()) {
        let id = format!("ENT_WPN_{}", p.rsplit('/').next().unwrap_or(p));
        let e = m.entities.get(&id).ok_or_else(|| format!("spec: no {id}"))?;
        let val = |k: &str| e.values.get(&format!("FLD_WPN_{k}")).cloned().unwrap_or_default();
        let sock = weapon_sockets_ue(&rd, p)?;
        let second = crate::physics::weapon_second_sockets_ue(&rd, p)?;
        let (clash_normal, second_clash_normal, clash_radius) = crate::physics::weapon_clash_ue(&rd, p);
        let rot = v3(&val("EQUIP_ROTATION_OFFSET"));
        // AMordhauWeapon::RecalculateTracerPoints rva=0x163a940 (decomp AMordhauWeapon.cpp 2767-2779): with
        // bDeriveHandGripFromTracers / bSecondDeriveHandGripFromTracers (both true in the ctor, decomp 2155-2156)
        // GripLocationLocal = TraceStart - normalize(TraceEnd - TraceStart) * 8 in the weapon mesh's component space
        // (GetSocketTransform RTS_Component), overriding the class default. A missing socket reads as the origin, and a
        // zero direction normalizes to ZeroVector (FVector::GetSafeNormal). Found by the reader method (2026-10-06):
        // the real game's 1P greatsword root sits 8.75 cm down the blade axis from the grip; ours used the CDO's 0.
        let defaults = rd.defaults(p);
        let flag = |k: &str| defaults.get(k).and_then(|v| v.as_bool()).unwrap_or(true);
        let derive = |a: Option<FVector>, b: Option<FVector>| -> FVector {
            let a = a.unwrap_or(FVector::ZERO);
            let b = b.unwrap_or(FVector::ZERO);
            let d = FVector::new(b.x - a.x, b.y - a.y, b.z - a.z);
            let l2 = d.x * d.x + d.y * d.y + d.z * d.z;
            let n = if l2 == 1.0 { d } else if l2 < 1e-8 { FVector::ZERO } else { let k = 1.0 / l2.sqrt(); FVector::new(d.x * k, d.y * k, d.z * k) };
            FVector::new(a.x - n.x * 8.0, a.y - n.y * 8.0, a.z - n.z * 8.0)
        };
        let ts = sock.map(|s| FVector::new(s.0[0], s.0[1], s.0[2]));
        let te = sock.map(|s| FVector::new(s.1[0], s.1[1], s.1[2]));
        let grip0 = if flag("bDeriveHandGripFromTracers") { derive(ts, te) } else { v3(&val("EQUIP_GRIP_LOCATION_LOCAL")) };
        let grip1 = if flag("bSecondDeriveHandGripFromTracers") { Some(derive(second.0, second.1)) } else { None };
        wg.insert(
            p.clone(),
            WeaponGeo {
                trace_start: sock.map(|s| FVector::new(s.0[0], s.0[1], s.0[2])),
                trace_end: sock.map(|s| FVector::new(s.1[0], s.1[1], s.1[2])),
                clash_normal,
                second_clash_normal,
                clash_radius,
                parry_box: crate::physics::weapon_parry_box(&rd, p),
                second_trace_start: second.0,
                second_trace_end: second.1,
                right_hand_equip_offset: v3(&val("EQUIP_RIGHT_HAND_EQUIP_OFFSET")),
                rotation_offset: [rot.x, rot.y, rot.z],
                grip_location_local: grip0,
                right_handed: val("EQUIP_B_IS_RIGHT_HANDED").as_bool().unwrap_or(true),
                equipped_offset: equipped_offset(&rd, p),
                block_box: crate::physics::weapon_block_box(&rd, p),
                // grip r2: both modes' grip fields from the class defaults (grip.rs; SwitchMode_Implementation
                // rva=0x156e280 swaps them)
                extra_environment_tracers: extra_environment_tracers(&rd, p, &defaults),
                path: p.clone(),
                grip_modes: Some({
                    // the primary mode as the spec records it (the native ctors included, e.g. a shield's
                    // bIsRightHanded), the Second* fields from the class defaults
                    let mut m = crate::grip::modes_from_cdo(&rd.defaults(p));
                    m[0].rotation_offset = [rot.x, rot.y, rot.z];
                    m[0].grip_location_local = grip0;
                    if let Some(g) = grip1 {
                        m[1].grip_location_local = g;
                    }
                    m[0].right_handed = val("EQUIP_B_IS_RIGHT_HANDED").as_bool().unwrap_or(true);
                    m[0].use_equipped_offset = equipped_offset(&rd, p).is_some();
                    m
                }),
                equipped_offset_xf: equipped_offset_raw(&rd.defaults(p)),
            },
        );
    }
    let c = |n: &str| -> Result<f32, String> {
        m.entities
            .get(&format!("ENT_CONST_{n}"))
            .and_then(|e| e.values.get("FLD_CONST_VALUE"))
            .and_then(|v| v.as_f64())
            .map(|x| x as f32)
            .ok_or_else(|| format!("spec: no constant {n}"))
    };
    let k = &spec.constants;
    let (bc_half, bc_centre) = crate::physics::block_collider_ue(&rd)?;
    let geo = Geometry {
        block_collider_rel: FVector::new(bc_centre[0], bc_centre[1], bc_centre[2]),
        block_collider_extent: FVector::new(bc_half[0], bc_half[1], bc_half[2]),
        block_collider_offsets: crate::physics::block_collider_offsets(&rd)?,
        shapes,
        shape_bones,
        skeleton,
        mesh_xf,
        weapons: wg,
        grip_pitch_right: c("PLAYER_grip_pitch_right")?,
        grip_pitch_left: c("PLAYER_grip_pitch_left")?,
        count_scale: k.tracer_count_scale as f32,
        round_bias: k.tracer_round_bias as f32,
        spacing_cm: k.tracer_spacing_cm as f32,
        step: k.tracer_step as f32,
    };
    Ok(Loaded { spec, geo })
}

/// Original AMordhauWeapon constructor0x1610e30 true, exact native subclass ctors0x14e35d0/0x14e3890 false.
/// Cooked inherited overrides win; special native identities are compared exactly, never by asset-name substring.
fn extra_environment_tracers(rd: &mh_pak::Reader, path: &str, defaults: &serde_json::Map<String, serde_json::Value>) -> bool {
    if let Some(value) = defaults.get("bUsesExtraEnvironmentTracers").and_then(|value| value.as_bool()) { return value; }
    let chain = rd.chain(path);
    let native = chain.last().map(|root| rd.super_of(root)).unwrap_or_default();
    !(native["ObjectPath"].as_str() == Some("/Script/Mordhau") &&
      matches!(native["ObjectName"].as_str(), Some("Class'FistsWeapon'") | Some("Class'KickWeapon'")))
}

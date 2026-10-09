//! Cascade particle systems from the paks (mh_assets::particles) against CUE4Parse's decode (extract/json) for the
//! combat effects: the weapon's parry / block / world-impact systems (BP_MordhauWeapon), armour sparks
//! (BP_ArmorBloodSplash) and the blood hits the blueprints reference (Particles/Blood). Per system: emitters, LODs, module
//! classes, materials, bursts and every baked distribution table value; the distributions evaluate within their
//! [MinValue, MaxValue] ranges at every table time.

use mh_assets::particles::{self, RandomStream};
use serde_json::Value;
use std::path::Path;
use std::sync::Arc;

const SYSTEMS: &[&str] = &[
    "Mordhau/Content/Mordhau/Particles/ParrySparks/P_spark_burst",
    "Mordhau/Content/Mordhau/Particles/ParrySparks/P_spark_slide",
    "Mordhau/Content/Mordhau/Particles/ParrySparks/P_BlockDust",
    "Mordhau/Content/Mordhau/Particles/Blood/P_ArmorSparks",
    "Mordhau/Content/Mordhau/Particles/Blood/P_BloodWeaponTrailBlood",
    "Mordhau/Content/Mordhau/Particles/Blood/P_WeaponTrailDistort",
    "Mordhau/Content/Mordhau/Particles/Blood/P_BloodSplash",
    "Mordhau/Content/Mordhau/Particles/Blood/P_BloodDripsFocused",
    "Mordhau/Content/Mordhau/Particles/Blood/P_BloodSquirtShort",
    "Mordhau/Content/Mordhau/Particles/WorldImpacts/P_dirt_burst",
    "Mordhau/Content/Mordhau/Particles/WorldImpacts/P_grass_burst",
    "Mordhau/Content/Mordhau/Particles/WorldImpacts/P_pebble_burst",
    "Mordhau/Content/Mordhau/Particles/WorldImpacts/P_wet_burst",
    "Mordhau/Content/Mordhau/Particles/WorldImpacts/P_wood_burst",
];

fn idx(v: &Value) -> usize {
    v["ObjectPath"].as_str().unwrap().rsplit_once('.').unwrap().1.parse().unwrap()
}

#[test]
fn combat_particle_systems_equal_cue4parse() {
    let rd = mh_pak::Reader::new(Arc::new(mh_pak::Vfs::mount_default().expect("mount")));
    let (mut modules, mut tables, mut evals) = (0, 0, 0);
    let mut rng = RandomStream(1234);
    for pkg in SYSTEMS {
        let ps = particles::read(&rd, pkg).unwrap_or_else(|| panic!("{pkg}: no ParticleSystem"));
        let f = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../../extract/json/{pkg}.json"));
        let d: Value = serde_json::from_slice(&std::fs::read(&f).unwrap()).unwrap();
        let ex = d.as_array().unwrap();
        let js = ex.iter().find(|e| e["Type"] == "ParticleSystem").unwrap();
        let je = js["Properties"]["Emitters"].as_array().unwrap();
        assert_eq!(ps.emitters.len(), je.len(), "{pkg}: emitters");
        for (em, jr) in ps.emitters.iter().zip(je) {
            let jl = ex[idx(jr)]["Properties"]["LODLevels"].as_array().unwrap();
            assert_eq!(em.lods.len(), jl.len(), "{pkg} {}: LODs", em.name);
            for (lod, lr) in em.lods.iter().zip(jl) {
                let lp = &ex[idx(lr)]["Properties"];
                let jm = lp["Modules"].as_array().unwrap();
                assert_eq!(lod.modules.len(), jm.len(), "{pkg}: modules");
                assert_eq!(lod.material(), lp.get("RequiredModule").map(|r| ex[idx(r)]["Properties"]["Material"]["ObjectPath"].as_str().unwrap_or("")).unwrap_or("").rsplit_once('.').map_or("", |x| x.0));
                let mut all: Vec<(&mh_assets::particles::Module, &Value)> = lod.modules.iter().zip(jm.iter().map(|r| &ex[idx(r)])).collect();
                for (m, r) in [(&lod.required, lp.get("RequiredModule")), (&lod.spawn, lp.get("SpawnModule"))] {
                    if let (Some(m), Some(r)) = (m, r) {
                        all.push((m, &ex[idx(r)]));
                    }
                }
                for (m, j) in all {
                    assert_eq!(m.class, j["Type"].as_str().unwrap(), "{pkg}");
                    modules += 1;
                    for (k, dv) in &m.dists {
                        let w = &j["Properties"][k];
                        if let Some(t) = &dv.table {
                            let want: Vec<f32> = w["Table"]["Values"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap() as f32).collect();
                            assert_eq!(t.values, want, "{pkg} {} {k}", m.name);
                            assert_eq!(t.op as i64, w["Table"]["Op"].as_i64().unwrap_or(0), "{pkg} {} {k} op", m.name);
                            tables += 1;
                            // evaluated within the cooked range at each entry time
                            if t.entry_stride == 1 || t.entry_stride == 2 {
                                for i in 0..t.entry_count.max(1) {
                                    let tt = t.time_bias + if t.time_scale > 0.0 { i as f32 / t.time_scale } else { 0.0 };
                                    let v = dv.value1(tt, &mut || rng.fraction());
                                    let (lo, hi) = (t.values.iter().cloned().fold(f32::MAX, f32::min), t.values.iter().cloned().fold(f32::MIN, f32::max));
                                    assert!(v >= lo - 1e-4 && v <= hi + 1e-4, "{pkg} {} {k}: {v} outside [{lo}, {hi}]", m.name);
                                    evals += 1;
                                }
                            }
                        }
                    }
                }
                if let Some(s) = lp.get("SpawnModule") {
                    let jb = ex[idx(s)]["Properties"]["BurstList"].as_array().map_or(0, |a| a.len());
                    assert_eq!(lod.bursts().len(), jb, "{pkg}: bursts");
                }
            }
        }
    }
    println!("  {} systems, {modules} modules, {tables} distribution tables = CUE4Parse, {evals} evaluations in range", SYSTEMS.len());
    assert!(tables > 100);
}

/// P_spark_burst, the parry spark: emitter 1 bursts 3..6 sprites, lifetime 0.2 s; emitter 2 lifetime uniform 0.5..1.5
#[test]
fn spark_burst_values() {
    let rd = mh_pak::Reader::new(Arc::new(mh_pak::Vfs::mount_default().expect("mount")));
    let ps = particles::read(&rd, SYSTEMS[0]).unwrap();
    let l0 = &ps.emitters[0].lods[0];
    assert_eq!(l0.bursts(), vec![(6, 3, 0.0)]);
    assert_eq!(l0.module("ParticleModuleLifetime").unwrap().dist("Lifetime").unwrap().value1(0.0, &mut || 0.9), 0.2);
    let life = ps.emitters[1].lods[0].module("ParticleModuleLifetime").unwrap().dist("Lifetime").unwrap();
    assert_eq!((life.value1(0.0, &mut || 0.0), life.value1(0.0, &mut || 0.5)), (0.5, 1.0));
    assert_eq!(l0.material(), "Mordhau/Content/Mordhau/Particles/ParrySparks/M_radial_ramp");
}

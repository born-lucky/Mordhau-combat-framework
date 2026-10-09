//! mh-fx coverage evidence (CPU only, no GPU): every ParticleSystemComponent placed in the FFA maps (map_fx.rs):
//! each Template loads (mh-assets particles), simulates 3 s (sim.rs) from its placed transform, its modules not
//! simulated are listed, and its emitters' materials are classified by material.rs: shader-derived mode, not drawn
//! (None), or the mode-0 fallback (no shader dump yet: UNCONFIRMED pixel math).
//! Output: state/runtime_evidence/<date>/<time>-mh-fx-coverage/coverage.json
//!   sh scripts/cargo.sh run -p mh-fx --example coverage

use mh_fx::{map_fx, material, sim::SystemSim};
use serde_json::json;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

fn main() {
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let rd = mh_pak::Reader::new(vfs.clone());
    let dir = mh_ui::evidence::run_dir("mh-fx-coverage");
    let mut maps: Vec<String> = vfs
        .list()
        .filter(|p| p.starts_with("Mordhau/Content/Mordhau/Maps/") && p.ends_with(".umap"))
        .filter(|p| p.rsplit('/').next().is_some_and(|n| n.starts_with("FFA_") && !n.contains("_64") && !n.contains("Legacy")))
        .map(|p| p.trim_end_matches(".umap").to_string())
        .collect();
    maps.sort();
    let mut systems: BTreeMap<String, serde_json::Value> = BTreeMap::new();
    let mut materials: BTreeMap<String, (String, usize)> = BTreeMap::new();
    let mut unsupported: BTreeMap<String, usize> = BTreeMap::new();
    let mut map_rows = vec![];
    for m in &maps {
        let pk = mh_level::level::Pkgs::new(mh_pak::Reader::new(vfs.clone()));
        let em = map_fx::map_emitters(&pk, m);
        let mut owners: BTreeMap<String, usize> = BTreeMap::new();
        let mut templates: BTreeSet<String> = BTreeSet::new();
        for e in &em {
            *owners.entry(e.owner.clone()).or_default() += 1;
            templates.insert(e.template.clone());
        }
        for t in &templates {
            if systems.contains_key(t) {
                continue;
            }
            let Some(ps) = mh_assets::particles::read(&rd, t) else {
                systems.insert(t.clone(), json!({"loaded": false}));
                continue;
            };
            let e0 = em.iter().find(|e| &e.template == t).unwrap();
            let mut s = SystemSim::new(&ps, e0.pos_ue.map(|x| x as f32), e0.dir_ue.map(|x| x as f32), 11);
            let mut peak = 0;
            let mut spawned_by_1s = 0;
            for i in 0..180 {
                s.step(1.0 / 60.0);
                peak = peak.max(s.alive());
                if i == 59 {
                    spawned_by_1s = s.emitters.iter().map(|e| e.spawned).sum();
                }
            }
            for u in &s.unsupported {
                *unsupported.entry(u.clone()).or_default() += 1;
            }
            let mut mats = vec![];
            for e in &s.emitters {
                let mode = material::mode_of(&e.material);
                let class = match mode {
                    None => "not drawn".to_string(),
                    Some((0, _, _)) => "fallback".to_string(),
                    Some((k, _, _)) => format!("mode {k}"),
                };
                let ent = materials.entry(e.material.clone()).or_insert((class.clone(), 0));
                ent.1 += 1;
                mats.push(json!({"emitter": e.name, "material": e.material, "class": class}));
            }
            systems.insert(t.clone(), json!({"loaded": true, "emitters": s.emitters.len(), "spawned_first_1s": spawned_by_1s, "peak_alive_3s": peak,
                "looping_or_live_at_3s": !s.done(), "unsupported": s.unsupported, "materials": mats}));
        }
        map_rows.push(json!({"map": m, "particle_components": em.len(), "auto_activate": em.iter().filter(|e| e.auto_activate).count(), "owners": owners, "templates": templates}));
        println!("{m}: {} particle components, {} templates", em.len(), templates.len());
        pk.clear();
    }
    let loaded = systems.values().filter(|v| v["loaded"] == true).count();
    let mats: BTreeMap<String, serde_json::Value> = materials.iter().map(|(k, (c, n))| (k.clone(), json!({"class": c, "emitters": n}))).collect();
    let fallback: Vec<&String> = materials.iter().filter(|(_, (c, _))| c == "fallback").map(|(k, _)| k).collect();
    let out = json!({"plugin": "mh-fx", "what": "map particle coverage (CPU sim, no GPU)", "maps": map_rows, "systems_total": systems.len(), "systems_loaded": loaded,
        "unsupported_modules": unsupported, "materials": mats, "materials_fallback": fallback, "systems": systems});
    let p = dir.join("coverage.json");
    let _ = std::fs::write(&p, serde_json::to_string_pretty(&out).unwrap());
    println!("mh-fx coverage: {} ({} maps, {} systems, {} loaded, {} materials, {} on the fallback)", p.display(), maps.len(), systems.len(), loaded, materials.len(), fallback.len());
}

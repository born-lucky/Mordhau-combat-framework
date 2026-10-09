//! Map ambience dump (offline, no device): for each map, every placed AudioComponent (ambient.rs map_sounds) with
//! its cue graph, routing (class / effective class volume / concurrency), attenuation and position; the random
//! spawners; the AudioVolumes; the streamed levels with their LevelStreaming flags; and every level actor whose
//! class mentions audio / ambience (Blueprint-driven ambience candidates).
//! Output: state/runtime_evidence/<date>/<time>-mh-audio-ambience/ambience.json
//!   sh scripts/cargo.sh run -p mh-audio --example ambience [map pkg ...]

use mh_assets::sound_cue::{self, CueNode, NodeData};
use mh_audio::ambient;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

fn tree(n: &CueNode) -> Value {
    let d = match &n.data {
        NodeData::WavePlayer { wave, looping, wave_volume, duration, .. } => json!({"wave": wave.rsplit('/').next(), "loop": looping, "vol": wave_volume, "dur": duration}),
        NodeData::Looping { loop_count, indefinitely } => json!({"count": loop_count, "forever": indefinitely}),
        NodeData::DistanceCrossFade { inputs } => json!({"inputs": inputs}),
        NodeData::Random { weights, no_repeat } => json!({"n": weights.len(), "no_repeat": no_repeat}),
        NodeData::Modulator { pitch_min, pitch_max, volume_min, volume_max } => json!({"p": [pitch_min, pitch_max], "v": [volume_min, volume_max]}),
        NodeData::Mixer { input_volume } => json!({"in": input_volume}),
        NodeData::Concatenator { input_volume } => json!({"in": input_volume}),
        NodeData::Delay { min, max } => json!({"d": [min, max]}),
        NodeData::Attenuation(a) => json!({"falloff": a.as_ref().map(|a| a.falloff_cm), "inner": a.as_ref().map(|a| a.inner_radius_cm)}),
        _ => json!({}),
    };
    let ch: Vec<Value> = n.children.iter().map(tree).collect();
    json!({"k": n.kind, "d": d, "c": ch})
}

fn att(a: &Option<sound_cue::Attenuation>) -> Value {
    a.as_ref().map_or(Value::Null, |a| {
        json!({"algo": a.algorithm, "shape": a.shape, "inner": a.inner_radius_cm, "falloff": a.falloff_cm, "attenuate": a.attenuate,
            "spatialize": a.spatialize, "omni": a.omni_radius_cm, "occlusion": a.enable_occlusion, "lpf": a.attenuate_with_lpf})
    })
}

fn main() {
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let rd = mh_pak::Reader::new(vfs.clone());
    let pk = mh_level::Pkgs::new(mh_pak::Reader::new(vfs.clone()));
    let tree_cls = mh_assets::sound_class::ClassTree::load(&rd);
    let eff = tree_cls.effective(&[]);
    let args: Vec<String> = std::env::args().skip(1).collect();
    let maps = if args.is_empty() {
        vec![
            "Mordhau/Content/Mordhau/Maps/Arena_Map/DU_Arena".to_string(),
            "Mordhau/Content/Mordhau/Maps/Contraband/FFA_Contraband".to_string(),
            "Mordhau/Content/Mordhau/Maps/Contraband/DU_Contraband".to_string(),
        ]
    } else {
        args
    };
    let mut out = vec![];
    for map in &maps {
        // levels with their streaming flags
        let mut lv = vec![];
        for l in mh_level::level::levels(&pk, map) {
            lv.push(json!({"pkg": l.pkg}));
        }
        let mut streaming = vec![];
        for l in mh_level::level::levels(&pk, map) {
            for e in pk.load_pkg(&l.pkg).iter() {
                let t = e.get("Type").and_then(Value::as_str).unwrap_or("");
                if t.starts_with("LevelStreaming") {
                    let p = pk.props(e);
                    streaming.push(json!({"in": l.pkg, "type": t, "world": p.get("WorldAsset"), "loaded": p.get("bShouldBeLoaded"), "visible": p.get("bShouldBeVisible"),
                        "initially_loaded": p.get("bInitiallyLoaded"), "initially_visible": p.get("bInitiallyVisible"), "always": t == "LevelStreamingAlwaysLoaded"}));
                }
            }
        }
        // audio-ish actor types per level
        let mut types: BTreeMap<String, usize> = BTreeMap::new();
        for l in mh_level::level::levels(&pk, map) {
            for e in pk.load_pkg(&l.pkg).iter() {
                let t = e.get("Type").and_then(Value::as_str).unwrap_or("");
                let tl = t.to_ascii_lowercase();
                if ["audio", "sound", "ambien", "wind", "bird", "crowd", "music", "reverb"].iter().any(|k| tl.contains(k)) {
                    *types.entry(t.to_string()).or_default() += 1;
                }
            }
        }
        let snd = ambient::map_sounds(&pk, map);
        let mut rows = vec![];
        for s in &snd {
            let c = sound_cue::cue(&rd, &s.sound);
            let r = mh_assets::sound_class::routing(&rd, &s.sound);
            let class = r.as_ref().map(|r| r.class.clone()).unwrap_or_default();
            let k = if class.is_empty() { mh_assets::sound_class::DEFAULT_CLASS.to_string() } else { class.clone() };
            let cv = eff.get(&k).map(|p| p.volume);
            let keep: serde_json::Map<String, Value> = s.props.iter().filter(|(k, _)| !["Sound", "RelativeLocation", "RelativeRotation", "AttachParent", "AttenuationOverrides"].contains(&k.as_str())).map(|(k, v)| (k.clone(), v.clone())).collect();
            rows.push(json!({
                "name": s.name, "owner": s.owner, "level": s.level.rsplit('/').next(), "cue": s.sound, "pos_ue": s.pos_ue,
                "auto": s.auto_activate, "volume": s.volume, "pitch": s.pitch, "component_att": att(&s.attenuation),
                "cue_att": c.as_ref().map(|c| att(&c.attenuation)), "cue_volume": c.as_ref().map(|c| c.volume), "exact": c.as_ref().map(|c| c.exact()),
                "class": class, "class_volume": cv,
                "concurrency": r.as_ref().map(|r| r.concurrency.iter().map(|c| json!({"path": c.path, "max": c.max_count, "rule": format!("{:?}", c.rule)})).collect::<Vec<_>>()),
                "max_distance": r.as_ref().and_then(|r| r.max_distance),
                "graph": c.as_ref().and_then(|c| c.root.as_ref()).map(tree), "props": keep,
            }));
        }
        let sp: Vec<Value> = ambient::random_spawners(&pk, map)
            .iter()
            .map(|s| json!({"name": s.name, "cue": s.sound, "pos_ue": s.pos_ue, "t": [s.time_min, s.time_max], "dist": s.distance, "h": s.max_height}))
            .collect();
        let ld = mh_level::read(&pk, map);
        let vols: Vec<Value> = ambient::audio_volumes(&pk, &ld)
            .iter()
            .map(|v| json!({"name": v.name, "prio": v.priority, "enabled": v.enabled, "reverb": v.reverb, "rv": v.reverb_volume, "zone": v.ambient_zone, "boxes": v.boxes.len()}))
            .collect();
        println!("{map}: {} levels, {} sounds ({} auto), {} spawners, {} volumes, audio types {:?}", lv.len(), snd.len(), snd.iter().filter(|s| s.auto_activate).count(), sp.len(), vols.len(), types);
        out.push(json!({"map": map, "levels": lv, "streaming": streaming, "audio_types": types, "sounds": rows, "spawners": sp, "volumes": vols}));
    }
    let dir = mh_ui::evidence::run_dir("mh-audio-ambience");
    let p = dir.join("ambience.json");
    std::fs::write(&p, serde_json::to_string_pretty(&out).unwrap()).unwrap();
    println!("wrote {}", p.display());
}

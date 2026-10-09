//! SoundCue graphs + attenuation from the paks (mh_assets::sound_cue) against the Godot port (ue_sound.gd UeSound.cue /
//! gain) for every SoundCue in the game, dumped by godot/tools/export_sound_cues.gd into data_gen/sounds/cues_godot.json:
//!   sh scripts/godot_import.sh --run timeout 2400 godot --headless --path godot --script res://tools/export_sound_cues.gd

use mh_assets::sound_cue::{cue, gain, CueNode, NodeData};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;

fn mc(m: &mh_assets::sound_cue::ContinuousParams) -> Value {
    json!({"param": m.param, "mode": m.mode, "Default": m.default, "MinInput": m.min_input, "MaxInput": m.max_input,
        "MinOutput": m.min_output, "MaxOutput": m.max_output})
}

/// our node in ue_sound.gd's dictionary shape
fn godot_shape(n: &CueNode) -> Value {
    if n.kind.is_empty() {
        return json!({});
    }
    let mut o = json!({"type": n.kind, "children": n.children.iter().map(godot_shape).collect::<Vec<_>>()});
    let m = o.as_object_mut().unwrap();
    match &n.data {
        NodeData::WavePlayer { wave, looping, .. } => {
            m.insert("wave".into(), json!(wave));
            m.insert("looping".into(), json!(looping));
        }
        NodeData::Modulator { pitch_min, pitch_max, volume_min, volume_max } => {
            for (k, v) in [("PitchMin", pitch_min), ("PitchMax", pitch_max), ("VolumeMin", volume_min), ("VolumeMax", volume_max)] {
                m.insert(k.into(), json!(v));
            }
        }
        NodeData::Random { weights, no_repeat } => {
            m.insert("weights".into(), json!(weights));
            m.insert("no_repeat".into(), json!(no_repeat));
        }
        NodeData::Mixer { input_volume } => {
            m.insert("input_volume".into(), json!(input_volume));
        }
        NodeData::Switch { param } | NodeData::Branch { param } => {
            m.insert("param".into(), json!(param));
        }
        NodeData::Delay { min, max } => {
            m.insert("DelayMin".into(), json!(min));
            m.insert("DelayMax".into(), json!(max));
        }
        NodeData::ModulatorContinuous { pitch, volume } => {
            m.insert("pitch_mod".into(), mc(pitch));
            m.insert("volume_mod".into(), mc(volume));
        }
        _ => {}
    }
    o
}

/// structural equality, numbers to 1e-6 (Godot packs float arrays as float32)
fn eq(a: &Value, b: &Value, path: &str, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => {
            let (x, y) = (x.as_f64().unwrap(), y.as_f64().unwrap());
            if (x - y).abs() > 1e-6 * (1.0 + y.abs()) {
                out.push(format!("{path}: pak {x} godot {y}"));
            }
        }
        (Value::Object(x), Value::Object(y)) => {
            for k in x.keys().chain(y.keys()) {
                match (x.get(k), y.get(k)) {
                    (Some(p), Some(q)) => eq(p, q, &format!("{path}.{k}"), out),
                    (p, q) => out.push(format!("{path}.{k}: pak {p:?} godot {q:?}")),
                }
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (p, q)) in x.iter().zip(y).enumerate() {
                eq(p, q, &format!("{path}[{i}]"), out);
            }
        }
        _ if a == b => {}
        _ => out.push(format!("{path}: pak {a} godot {b}")),
    }
}

#[test]
fn sound_cues_equal_godot() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../data_gen/sounds/cues_godot.json");
    let Ok(raw) = std::fs::read(&p) else { panic!("{} missing: run godot/tools/export_sound_cues.gd", p.display()) };
    let d: Value = serde_json::from_slice(&raw).unwrap();
    let rd = mh_pak::Reader::new(Arc::new(mh_pak::Vfs::mount_default().expect("mount the paks")));
    let dist: Vec<f64> = d["dist_m"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect();
    let mut diffs = Vec::new();
    let (mut n, mut inexact, mut att, mut customs, mut played) = (0, 0, 0, 0, 0);
    for (i, g) in d["cues"].as_array().unwrap().iter().enumerate() {
        let path = g["path"].as_str().unwrap();
        let c = cue(&rd, path);
        if g["empty"] == true {
            if c.is_some() {
                diffs.push(format!("{path}: Godot found no SoundCue"));
            }
            continue;
        }
        let Some(c) = c else {
            diffs.push(format!("{path}: no SoundCue from the paks"));
            continue;
        };
        n += 1;
        inexact += (g["exact"].as_bool() == Some(false)) as usize;
        // every node type is ported now (r4); Godot follows the first child of the six it does not port, so the
        // "unknown" lists differ by design and only the graph / values are compared
        if !c.unknown.is_empty() {
            diffs.push(format!("{path}: node types not ported {:?}", c.unknown));
        }
        // every cue evaluates (one play at 12.34 m, fixed random stream; at exactly FadeInDistanceStart = End the exe divides
        // 0 / 0 too - SC_Mortar_Impact at 1000 cm - so an exact boundary distance is avoided)
        let mut k = 0u32;
        let mut rng = || { k = k.wrapping_mul(1664525).wrapping_add(1013904223); (k >> 8) as f64 / (1u32 << 24) as f64 };
        let ctx = mh_assets::sound_cue::EvalCtx { params: Default::default(), distance_cm: Some(1234.0) };
        let plays = mh_assets::sound_cue::evaluate_ctx(&c, &ctx, &mut rng, &mut Default::default());
        if plays.iter().any(|p| !p.volume.is_finite() || !p.pitch.is_finite() || p.delay < 0.0) {
            diffs.push(format!("{path}: bad play {plays:?}"));
        }
        played += !plays.is_empty() as usize;
        let mut mine = json!({"volume": c.volume, "pitch": c.pitch, "waves": c.waves,
            "root": c.root.as_ref().map_or(json!({}), godot_shape)});
        let mut want = json!({"volume": g["volume"], "pitch": g["pitch"], "waves": g["waves"], "root": g["root"]});
        if let Some(a) = &c.attenuation {
            att += 1;
            let custom = a.algorithm == "Custom";
            mine["att"] = json!({"algorithm": a.algorithm, "shape": a.shape, "falloff_mode": a.falloff_mode,
                "inner_radius_m": a.inner_radius_cm * 0.01, "falloff_m": a.falloff_cm * 0.01, "db_at_max": a.db_at_max,
                "bAttenuate": a.attenuate, "bSpatialize": a.spatialize, "bAttenuateWithLPF": a.attenuate_with_lpf,
                "LPFRadiusMin": a.lpf_radius_min, "LPFRadiusMax": a.lpf_radius_max, "LPFFrequencyAtMin": a.lpf_frequency_at_min,
                "LPFFrequencyAtMax": a.lpf_frequency_at_max,
                "gain": dist.iter().map(|m| gain(Some(a), m * 100.0)).collect::<Vec<_>>()});
            if custom {
                // ue_sound.gd does not port the Custom curve (gain 1); mh-assets evaluates CustomAttenuationCurve
                customs += 1;
                mine["att"]["gain"] = g["attenuation"]["gain"].clone();
            }
        }
        if g["attenuation"].as_object().is_some_and(|o| !o.is_empty()) {
            want["att"] = g["attenuation"].clone();
        }
        let before = diffs.len();
        eq(&mine, &want, path, &mut diffs);
        if diffs.len() > before + 5 {
            diffs.truncate(before + 5);
        }
        if i % 500 == 499 {
            rd.clear_cache();
        }
    }
    println!("  {n} cues ({att} with attenuation, {customs} custom curves, {inexact} with nodes Godot does not port, {played} with a wave in one play at 12.34 m), {} differences", diffs.len());
    for x in diffs.iter().take(60) {
        println!("  DIFF {x}");
    }
    assert!(n > 100, "only {n} cues");
    assert!(diffs.is_empty(), "{} differences", diffs.len());
}

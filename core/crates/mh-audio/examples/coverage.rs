//! mh-audio coverage evidence (offline, no device, no window): beyond combat.
//! - every FFA map: its placed AudioComponents (AmbientSound + Blueprint components, ambient.rs): each sound loads,
//!   its cue graph is fully understood (exact), its waves decode (mixer::decode_ogg); its routing (class /
//!   concurrency / submix); the map's AudioVolumes (priority, reverb effect, ambient zone) and random spawners
//! - footsteps: SC_HumanFootstep evaluated per Surface (sources.rs) -> the waves each surface plays
//! - voice packs: every Blueprints/Voices pack's event cues load, are exact, and decode
//! - sound classes: the effective class volumes (class tree, no mix) and the passive mixes they push
//! - concurrency: 12 hits at equal distance into WeaponDamageConcurrency (MaxCount 8, StopFarthestThenOldest)
//! Output: state/runtime_evidence/<date>/<time>-mh-audio-coverage/coverage.json
//!   sh scripts/cargo.sh run -p mh-audio --example coverage

use mh_assets::sound_cue::{self, CueState, EvalCtx};
use mh_audio::{ambient, mixer, routing, sources};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

struct Waves {
    src: mh_assets::pak_source::PakSource,
    done: BTreeMap<String, (bool, f64, u16, u32)>,
}

impl Waves {
    /// decode once: (ok, seconds, channels, rate)
    fn check(&mut self, w: &str) -> (bool, f64, u16, u32) {
        if let Some(x) = self.done.get(w) {
            return *x;
        }
        let r = mh_assets::sound::ogg(&self.src, w).ok().and_then(|b| mixer::decode_ogg(&b).ok()).map_or((false, 0.0, 0, 0), |p| (p.frames() > 0, p.seconds(), p.channels, p.rate));
        self.done.insert(w.to_string(), r);
        r
    }
}

fn main() {
    let vfs = Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let rd = mh_pak::Reader::new(vfs.clone());
    let mut waves = Waves { src: mh_assets::pak_source::PakSource::new(vfs.clone()), done: BTreeMap::new() };
    let dir = mh_ui::evidence::run_dir("mh-audio-coverage");
    let mut rng = mh_assets::particles::RandomStream(7);
    let mut cue_cache: BTreeMap<String, Option<sound_cue::SoundCue>> = BTreeMap::new();
    let mut ok_all = true;

    // waves a sound can play: a cue's graph waves (all of them, not one evaluation), or the wave itself
    let sound_waves = |s: &str, cues: &mut BTreeMap<String, Option<sound_cue::SoundCue>>| -> (bool, bool, Vec<String>) {
        if !cues.contains_key(s) {
            cues.insert(s.to_string(), sound_cue::cue(&rd, s));
        }
        match &cues[s] {
            Some(c) => (true, c.exact(), c.waves.clone()),
            None => (false, true, vec![s.to_string()]),
        }
    };

    // 1. maps
    let mut maps: Vec<String> = vfs
        .list()
        .filter(|p| p.starts_with("Mordhau/Content/Mordhau/Maps/") && p.ends_with(".umap"))
        .filter(|p| p.rsplit('/').next().is_some_and(|n| n.starts_with("FFA_") && !n.contains("_64") && !n.contains("Legacy")))
        .map(|p| p.trim_end_matches(".umap").to_string())
        .collect();
    maps.sort();
    let mut map_rows = vec![];
    for m in &maps {
        let pk = mh_level::level::Pkgs::new(mh_pak::Reader::new(vfs.clone()));
        let sounds = ambient::map_sounds(&pk, m);
        let mut owners: BTreeMap<String, usize> = BTreeMap::new();
        let mut distinct: BTreeSet<String> = BTreeSet::new();
        for s in &sounds {
            *owners.entry(s.owner.clone()).or_default() += 1;
            distinct.insert(s.sound.clone());
        }
        let (mut cues_ok, mut exact, mut wave_ok, mut wave_bad, mut secs) = (0, 0, 0, 0, 0.0);
        let mut bad = vec![];
        let mut classes: BTreeMap<String, usize> = BTreeMap::new();
        for s in &distinct {
            let (is_cue, ex, ws) = sound_waves(s, &mut cue_cache);
            cues_ok += is_cue as usize;
            exact += ex as usize;
            if let Some(r) = mh_assets::sound_class::routing(&rd, s) {
                *classes.entry(r.class.rsplit('/').next().unwrap_or("").to_string()).or_default() += 1;
            }
            for w in ws {
                let (ok, t, _, _) = waves.check(&w);
                if ok {
                    wave_ok += 1;
                    secs += t;
                } else {
                    wave_bad += 1;
                    bad.push(w);
                }
            }
        }
        // AudioVolumes (mh-level volumes + actor properties) and random audio spawners
        let ld = mh_level::level::read(&pk, m);
        let av = ambient::audio_volumes(&pk, &ld);
        let vols: Vec<Value> = av
            .iter()
            .map(|v| {
                let (lo, hi) = v.boxes[0];
                let c = [(lo[0] + hi[0]) / 2.0, (lo[1] + hi[1]) / 2.0, (lo[2] + hi[2]) / 2.0];
                let at = ambient::volume_at(&av, c).map(|x| x.name.clone());
                json!({"name": v.name, "priority": v.priority, "enabled": v.enabled, "reverb": v.reverb, "reverb_volume": v.reverb_volume,
                    "fade": v.reverb_fade, "ambient_zone": v.ambient_zone, "boxes": v.boxes.len(), "volume_at_its_center": at})
            })
            .collect();
        let mut spawners = 0;
        for lv in &ld.levels {
            for e in pk.load_pkg(&lv.pkg).iter() {
                if e.get("Type").and_then(Value::as_str).unwrap_or("").contains("RandomAudioSpawner") {
                    spawners += 1;
                }
            }
        }
        ok_all &= wave_bad == 0;
        map_rows.push(json!({"map": m, "audio_components": sounds.len(), "auto_activate": sounds.iter().filter(|s| s.auto_activate).count(),
            "owners": owners, "distinct_sounds": distinct.len(), "cues": cues_ok, "cue_graphs_exact": exact, "waves_decoded": wave_ok,
            "waves_failed": wave_bad, "failed": bad, "decoded_seconds": secs, "classes": classes, "audio_volumes": vols, "random_spawners": spawners}));
        println!("{m}: {} components, {} sounds, {} waves ok, {} bad", sounds.len(), distinct.len(), wave_ok, wave_bad);
        pk.clear();
    }

    // 2. footsteps per surface
    let fs = sources::Footsteps::read(&rd);
    let mut foot = vec![];
    if let Some(c) = sound_cue::cue(&rd, &fs.cue) {
        for (si, sname) in sources::SURFACES.iter().enumerate() {
            let mut ws: BTreeSet<String> = BTreeSet::new();
            let mut st = CueState::default();
            for i in 0..24 {
                let (params, _, _) = fs.step(si, 250.0, (i % 3) as i64, false, sources::Relation::Enemy);
                let plays = sound_cue::evaluate_ctx(&c, &EvalCtx { params, distance_cm: Some(500.0) }, &mut || rng.fraction() as f64, &mut st);
                for p in plays {
                    ws.insert(p.wave.rsplit('/').nth(1).unwrap_or("").to_string() + "/" + p.wave.rsplit('/').next().unwrap_or(""));
                }
            }
            foot.push(json!({"surface": si, "name": sname, "waves": ws, "particles": fs.particles.get(si)}));
        }
    }
    let (_, v250, p250) = fs.step(5, 250.0, 0, false, sources::Relation::Enemy);

    // 3. voice packs
    let packs = sources::VoicePack::all(&rd);
    let mut voice_rows = vec![];
    for vp in &packs {
        let mut evs = vec![];
        for (ev, cue) in &vp.cues {
            let (is_cue, ex, ws) = sound_waves(cue, &mut cue_cache);
            let mut okw = 0;
            let mut badw = 0;
            for w in ws.iter().take(4) {
                if waves.check(w).0 {
                    okw += 1;
                } else {
                    badw += 1;
                }
            }
            ok_all &= is_cue && badw == 0;
            evs.push(json!({"event": ev, "cue": cue, "loaded": is_cue, "exact": ex, "waves": ws.len(), "decoded_first4_ok": okw, "decoded_bad": badw}));
        }
        voice_rows.push(json!({"pack": vp.bp, "name": vp.name, "pitch_limits": vp.pitch_limits, "events": evs}));
    }

    // 4. classes
    let tree = mh_assets::sound_class::ClassTree::load(&rd);
    let eff = tree.effective(&[]);
    let mut cls: BTreeMap<String, Value> = BTreeMap::new();
    for (k, c) in &tree.classes {
        let e = eff[k];
        cls.insert(k.rsplit('/').next().unwrap_or("").to_string(), json!({"own_volume": c.props.volume, "effective_volume": e.volume, "effective_pitch": e.pitch,
            "parent": c.parent.rsplit('/').next(), "passive_mixes": c.passive.iter().map(|p| p.mix.rsplit('/').next().unwrap_or("").to_string()).collect::<Vec<_>>()}));
    }
    let heart = "Mordhau/Content/Mordhau/Audio/Other/Mix/HeartbeatDuckingMix".to_string();
    let ducked = tree.effective(&[(heart.clone(), 1.0)]);
    let pre_ambient = "Mordhau/Content/Mordhau/Audio/Other/Classes/PreMixClasses/PreAmbient";
    let duck_row = json!({"mix": heart, "PreAmbient_volume_without": eff.get(pre_ambient).map(|x| x.volume), "PreAmbient_volume_with": ducked.get(pre_ambient).map(|x| x.volume)});

    // 5. concurrency: 12 equal-distance hits into WeaponDamageConcurrency
    let wd = mh_assets::sound_class::concurrency(&rd, "Mordhau/Content/Mordhau/Audio/Other/Concurrency/WeaponDamageConcurrency").expect("concurrency");
    let mut members: Vec<routing::Member> = vec![];
    let mut verdicts = vec![];
    for i in 0..12u64 {
        let v = routing::evaluate(&wd, &members, i as f64 * 0.01, 300.0);
        if let routing::Verdict::Play { stop } = &v {
            members.retain(|m| !stop.contains(&m.inst));
            members.push(routing::Member { inst: i, start: i as f64 * 0.01, dist_cm: 300.0 });
        }
        verdicts.push(format!("{v:?}"));
    }

    let out = json!({
        "plugin": "mh-audio", "what": "coverage beyond combat (offline)",
        "maps": map_rows,
        "footsteps": {"cue": fs.cue, "volume_in": fs.volume_in, "volume_out": fs.volume_out, "pitch_in": fs.pitch_in, "pitch_out": fs.pitch_out,
            "modifiers": [fs.mod_view_target, fs.mod_ally, fs.mod_enemy], "grass_250cm_s_enemy": {"volume": v250, "pitch": p250}, "per_surface": foot},
        "voice_packs": voice_rows,
        "classes": {"root": tree.root, "count": tree.classes.len(), "mixes": tree.mixes.keys().collect::<Vec<_>>(), "per_class": cls, "heartbeat_ducking": duck_row},
        "concurrency_demo": {"asset": wd.path, "max_count": wd.max_count, "rule": format!("{:?}", wd.rule), "verdicts": verdicts, "playing_at_end": members.len()},
        "waves_checked": waves.done.len(), "all_waves_ok": ok_all,
    });
    let p = dir.join("coverage.json");
    let _ = std::fs::write(&p, serde_json::to_string_pretty(&out).unwrap());
    println!("mh-audio coverage: {} ({} maps, {} voice packs, {} waves checked, all ok {ok_all})", p.display(), maps.len(), packs.len(), waves.done.len());
}

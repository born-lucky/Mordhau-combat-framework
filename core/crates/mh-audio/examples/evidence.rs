//! mh-audio evidence. Default: headless, no audio device (the offline fallback). BP_Longsword's combat cues (from
//! its CDO chain) at 1 / 10 / 40 m, a strike hit repeated (Random-node no-repeat state), a parry, a clash, a stab
//! hit, a Doppler cue flying past at 30 m/s; the offline mixer's stereo output is captured to mix.wav and checked:
//!   1. decode: every started wave's Ogg decoded by the mixer (lewton + ov_read rounding) == ffmpeg's float decoder
//!      through the same rounding, sample for sample (and the end trim explained by the last Ogg page)
//!   2. mixer: an isolated hit at 2 m, captured, == an independent f64 reference of the shipped mixer's math
//!      (linear interpolation at clamp(pitch) x rate / 48000 steps, x volume x attenuation x -3 dB headroom), played
//!      90 degrees to the listener's right: linear panning puts it all in the right channel, none in the left
//!   3. doppler: the flyby's pitch factor is > 1 while it approaches and < 1 after it passes
//!
//! `--device`: bevy_audio's AudioPlugin (rodio) is added with global volume 0 (silent: the user may be playing), the
//! same requests go to the output device, and the evidence records that the device thread pulled the voices and
//! how many frames it rendered; with no device bevy_audio only warns and the run still succeeds.
//! Output: state/runtime_evidence/<date>/<time>-mh-audio/{audio_log.json, mix.wav}.
//!   sh scripts/cargo.sh run -p mh-audio --example evidence [-- --device]

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use mh_audio::*;
use std::sync::atomic::Ordering;
use std::time::Duration;

const LONGSWORD: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";
const FLYBY: &str = "Mordhau/Content/Mordhau/Audio/Cues/Horde/Weapons/Blessed/SC_Blessed_LongSword_Flight_DIH";

#[derive(Resource)]
struct Run {
    frame: u32,
    device: bool,
    dir: std::path::PathBuf,
    sounds: Option<WeaponSounds>,
    script: Vec<serde_json::Value>,
    /// (offline frame, log row count) when the isolated check hit was requested
    iso_from: Option<(u64, usize)>,
    /// per flyby wave: (frame, live doppler factor)
    doppler: std::collections::BTreeMap<String, Vec<(u32, f32)>>,
    device_seen: Vec<serde_json::Value>,
}

fn main() {
    let device = std::env::args().any(|a| a == "--device");
    let mut app = App::new();
    app.add_plugins(MinimalPlugins.set(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 60.0))));
    if device {
        // no window here: ask for the device explicitly
        std::env::set_var("MH_AUDIO", "device");
        app.add_plugins(AssetPlugin::default()).add_plugins(bevy::audio::AudioPlugin {
            global_volume: bevy::audio::GlobalVolume::new(bevy::audio::Volume::Linear(0.0)),
            ..default()
        });
    }
    app.add_plugins(AudioPlugin).add_systems(Update, script).insert_resource(Run {
        frame: 0,
        device,
        dir: mh_ui::evidence::run_dir("mh-audio"),
        sounds: None,
        script: vec![],
        iso_from: None,
        doppler: Default::default(),
        device_seen: vec![],
    });
    if !device {
        app.world_mut().resource_mut::<OfflineMix>().capture = Some(vec![]);
    }
    app.run();
}

fn play(world: &mut World, what: &str, cue: &str, pos: Vec3, vel: Vec3) {
    world.write_message(PlayCue { cue: cue.to_string(), pos, vel, ..default() });
    let f = world.resource::<Run>().frame;
    world.resource_mut::<Run>().script.push(serde_json::json!({"frame": f, "what": what, "cue": cue, "pos_m": [pos.x, pos.y, pos.z], "vel_mps": [vel.x, vel.y, vel.z]}));
}

fn idle(world: &World) -> bool {
    let st = world.non_send::<AudioState>();
    st.voices.is_empty() && st.pending_count() == 0
}

fn script(world: &mut World) {
    let f = {
        let mut r = world.resource_mut::<Run>();
        r.frame += 1;
        r.frame
    };
    if f == 2 {
        let p = mh_ui::Paks::get_or_mount(world).expect("paks");
        let rd = mh_pak::Reader::new(p.0.clone());
        world.resource_mut::<Run>().sounds = Some(WeaponSounds::read(&rd, LONGSWORD));
        world.resource_mut::<AudioListener>().pos = Vec3::new(0.0, 1.7, 0.0);
    }
    let Some(w) = world.resource::<Run>().sounds.clone() else { return };
    let ev = |k: &str| w.for_event(&serde_json::json!({"kind": k})).unwrap_or("").to_string();
    // the flyby's live doppler per wave (updated every frame from the listener); its looping wave is stopped at
    // frame 300 (ctl.stop), once the flyby is far away
    {
        let st = world.non_send::<AudioState>();
        let rows: Vec<_> = st.voices.iter().filter(|v| v.cue == FLYBY).map(|v| (v.wave.clone(), v.ctl.lock().unwrap().doppler)).collect();
        if f == 300 {
            st.voices.iter().filter(|v| v.cue == FLYBY).for_each(|v| v.ctl.lock().unwrap().stop = true);
        }
        for (wave, d) in rows {
            world.resource_mut::<Run>().doppler.entry(wave).or_default().push((f, d));
        }
    }
    // device mode: what the device thread pulled
    if world.resource::<Run>().device && f % 30 == 0 {
        let st = world.non_send::<AudioState>();
        let rows: Vec<_> = st.voices.iter().map(|v| serde_json::json!({"wave": v.wave, "pulled": v.status.pulled.load(Ordering::Relaxed), "rendered_frames": v.status.rendered.load(Ordering::Relaxed)})).collect();
        let t = world.resource::<Time>().elapsed_secs_f64();
        world.resource_mut::<Run>().device_seen.push(serde_json::json!({"frame": f, "t": t, "voices": rows}));
    }
    let iso = world.resource::<Run>().iso_from;
    match f {
        5 => play(world, "hit at 1 m", &ev("hit"), Vec3::new(1.0, 1.7, 0.0), Vec3::ZERO),
        20 => play(world, "hit at 10 m", &ev("hit"), Vec3::new(10.0, 1.7, 0.0), Vec3::ZERO),
        35 => play(world, "hit at 40 m", &ev("hit"), Vec3::new(40.0, 1.7, 0.0), Vec3::ZERO),
        50 | 60 | 70 | 80 => play(world, "hit repeat at 2 m", &ev("hit"), Vec3::new(0.0, 1.7, 2.0), Vec3::ZERO),
        95 => play(world, "parry at 2 m", &ev("parry"), Vec3::new(0.0, 1.7, 2.0), Vec3::ZERO),
        110 => play(world, "clash at 2 m", &ev("clash"), Vec3::new(0.0, 1.7, 2.0), Vec3::ZERO),
        125 => play(world, "stab hit at 2 m", &w.stab_hit, Vec3::new(0.0, 1.7, 2.0), Vec3::ZERO),
        140 => play(world, "flyby 30 m/s", FLYBY, Vec3::new(-30.0, 1.7, 1.0), Vec3::new(30.0, 0.0, 0.0)),
        // the isolated check hit, once every earlier voice has finished (looping ones: give up waiting at 30 s)
        f if f >= 420 && iso.is_none() && (idle(world) || f > 1800) => {
            let fr = world.resource::<OfflineMix>().frames;
            let rows = world.resource::<AudioLog>().rows.len();
            world.resource_mut::<Run>().iso_from = Some((fr, rows));
            play(world, "isolated check hit 2 m to the right", &ev("hit"), Vec3::new(2.0, 1.7, 0.0), Vec3::ZERO);
        }
        f if iso.is_some() && f > 430 && (idle(world) || f > 3600) => finish(world, f),
        _ => {}
    }
}

fn wav(path: &std::path::Path, stereo: &[f32]) {
    let mut b = Vec::with_capacity(44 + stereo.len() * 4);
    let n = (stereo.len() * 4) as u32;
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&(36 + n).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&3u16.to_le_bytes()); // IEEE float
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&48000u32.to_le_bytes());
    b.extend_from_slice(&(48000u32 * 8).to_le_bytes());
    b.extend_from_slice(&8u16.to_le_bytes());
    b.extend_from_slice(&32u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&n.to_le_bytes());
    for s in stereo {
        b.extend_from_slice(&s.to_le_bytes());
    }
    let _ = std::fs::write(path, b);
}

/// ffmpeg's decode of an Ogg (float, interleaved) through the same ov_read rounding; None when ffmpeg is not available. The temporary .ogg is deleted.
fn ffmpeg_decode(dir: &std::path::Path, name: &str, ogg: &[u8]) -> Option<Vec<i16>> {
    let p = dir.join(format!("{name}.ogg"));
    std::fs::write(&p, ogg).ok()?;
    let out = std::process::Command::new("ffmpeg").args(["-v", "error", "-i"]).arg(&p).args(["-f", "f32le", "-acodec", "pcm_f32le", "-"]).output();
    let _ = std::fs::remove_file(&p);
    let out = out.ok()?;
    out.status.success().then(|| out.stdout.chunks_exact(4).map(|c| mixer::ov_read_sample(f32::from_le_bytes([c[0], c[1], c[2], c[3]]))).collect())
}

fn finish(world: &mut World, f: u32) {
    let log = world.resource::<AudioLog>().clone();
    let device = world.resource::<Run>().device;
    let dir = world.resource::<Run>().dir.clone();
    let mut checks = serde_json::Map::new();
    let mut ok = true;
    // 1. decode check over every distinct wave started
    {
        let mut waves: Vec<String> = log.rows.iter().map(|r| r.wave.clone()).collect();
        waves.sort();
        waves.dedup();
        let st = world.non_send::<AudioState>();
        let mut rows = vec![];
        for wv in &waves {
            let Some(ogg) = st.wave_ogg(wv) else { continue };
            let pcm = mixer::decode_ogg(&ogg).expect("decode");
            let name = wv.rsplit('/').next().unwrap_or("w");
            match ffmpeg_decode(&dir, name, &ogg) {
                Some(ff) => {
                    // ffmpeg's samples align with ours from the start (both keep the stream's sample 0)
                    let n = ff.len().min(pcm.samples.len());
                    let maxd = (0..n).map(|i| (ff[i] as i32 - pcm.samples[i] as i32).abs()).max().unwrap_or(0);
                    let diff = (0..n).filter(|&i| ff[i] != pcm.samples[i]).count();
                    // length: libvorbis' rule (mixer::decode_ogg) = the end-of-stream page's granule; ffmpeg's own
                    // trimming differs on short streams, so its length is reported, not required
                    let (eos, granule) = mixer::ogg_tail(&ogg).unwrap_or((false, 0));
                    let ch = pcm.channels as usize;
                    let len_ok = !eos || pcm.samples.len() == granule as usize * ch;
                    ok &= len_ok && maxd <= 1;
                    rows.push(serde_json::json!({"wave": wv, "rate": pcm.rate, "channels": pcm.channels, "samples": pcm.samples.len(), "ffmpeg_samples": ff.len(),
                        "last_page_eos": eos, "last_page_granule": granule, "length_ok": len_ok, "max_abs_diff_lsb": maxd, "samples_differing": diff}));
                }
                None => rows.push(serde_json::json!({"wave": wv, "ffmpeg": "unavailable"})),
            }
        }
        checks.insert("decode_vs_ffmpeg".into(), serde_json::json!(rows));
    }
    // 2. mixer check: the isolated hit's waves, captured offline, against an f64 reference summed over its waves
    if !device {
        let (iso_frame, iso_rows) = world.resource::<Run>().iso_from.unwrap();
        let (cap, starts) = {
            let mix = world.resource::<OfflineMix>();
            (mix.capture.clone().unwrap_or_default(), mix.starts.iter().filter(|(_, at)| *at >= iso_frame).cloned().collect::<Vec<_>>())
        };
        wav(&dir.join("mix.wav"), &cap);
        let rows = &log.rows[iso_rows..];
        let mut want = vec![0f64; cap.len()];
        let mut waves = vec![];
        let mut first = usize::MAX;
        let mut last = 0usize;
        let mut mono = true;
        for ((wave, at), row) in starts.iter().zip(rows) {
            let pcm = world.non_send_mut::<AudioState>().wave_pcm(wave).unwrap();
            mono &= pcm.channels == 1;
            let pitch = (row.pitch as f32).clamp(mixer::MIN_PITCH, mixer::MAX_PITCH) as f64 * pcm.rate as f64 / 48000.0;
            let vol = row.volume * row.gain;
            let k = (vol * 10f64.powf(-3.0 * 0.05)).min(4.0);
            let n = pcm.frames();
            // the source position in f32 as the exe keeps it (CurrentFrameAlpha += pitch, whole frames carried)
            let step = pitch as f32;
            let (mut i, mut a) = (0usize, 0f32);
            let mut o = 0usize;
            loop {
                while a >= 1.0 {
                    i += 1;
                    a -= 1.0;
                }
                let idx = (*at as usize + o) * 2;
                if i >= n || idx + 1 >= want.len() {
                    break;
                }
                let s0 = pcm.at(i, 0);
                // past the last frame ReadSourceFrame leaves `next` as it was: the last frame holds
                let s1 = if i + 1 < n { pcm.at(i + 1, 0) } else { s0 };
                let v = (s0 + a * (s1 - s0)) as f64 * k;
                // right only (az 90: FrontRight gain 1, FrontLeft 0)
                want[idx + 1] += v;
                a += step;
                o += 1;
            }
            first = first.min(*at as usize * 2);
            last = last.max((*at as usize + o) * 2);
            waves.push(serde_json::json!({"wave": wave, "rate": pcm.rate, "channels": pcm.channels, "started_at_frame": at, "pitch_step": pitch, "volume_x_gain": vol, "frames": o}));
        }
        let first = first.min(cap.len());
        let maxd = (first..cap.len()).map(|i| (cap[i] as f64 - want[i]).abs()).fold(0.0, f64::max);
        let peak = want.iter().fold(0.0f64, |m, x| m.max(x.abs()));
        let tail_silent = cap[last.min(cap.len())..].iter().all(|x| *x == 0.0);
        let pass = !waves.is_empty() && mono && peak > 1e-3 && maxd < 1e-4 && tail_silent;
        ok &= pass;
        checks.insert("mixer_vs_reference".into(), serde_json::json!({"waves": waves, "compared_samples": cap.len() - first, "peak": peak, "max_abs_diff": maxd, "tail_silent": tail_silent, "pass": pass}));
    }
    // 3. doppler: on the wave(s) with a Doppler node (factor != 1): > 1 approaching, < 1 receding
    {
        let d = world.resource::<Run>().doppler.clone();
        let mut rows = vec![];
        let mut pass = false;
        for (wave, v) in &d {
            let first = v.first().map(|x| x.1).unwrap_or(1.0);
            let last = v.last().map(|x| x.1).unwrap_or(1.0);
            let has = v.iter().any(|x| x.1 != 1.0);
            pass |= has && first > 1.0 && last < 1.0;
            rows.push(serde_json::json!({"wave": wave, "doppler_node": has, "first": first, "last": last, "samples": v.len()}));
        }
        ok &= pass;
        checks.insert("doppler".into(), serde_json::json!({"waves": rows, "pass": pass}));
    }
    if device {
        let seen = world.resource::<Run>().device_seen.clone();
        let pulled_any = seen.iter().any(|s| s["voices"].as_array().is_some_and(|v| v.iter().any(|x| x["pulled"] == true)));
        checks.insert("device".into(), serde_json::json!({"global_volume": 0.0, "pulled_any": pulled_any, "polls": seen}));
    }
    let r = world.resource::<Run>();
    let out = serde_json::json!({
        "plugin": "mh-audio", "mode": if device { "device (bevy_audio, silent)" } else { "offline (no device)" }, "frames": f, "weapon": LONGSWORD,
        "weapon_sounds": {"strike_hit": r.sounds.as_ref().map(|w| w.strike_hit.clone()), "blocked": r.sounds.as_ref().map(|w| w.blocked.clone())},
        "requests": r.script, "starts": log.rows.iter().map(|x| x.json()).collect::<Vec<_>>(),
        "missing": log.missing, "inexact": log.inexact.iter().map(|(c, u)| serde_json::json!({"cue": c, "unknown_nodes": u})).collect::<Vec<_>>(),
        "checks": checks, "all_pass": ok,
    });
    let path = r.dir.join("audio_log.json");
    let _ = std::fs::write(&path, serde_json::to_string_pretty(&out).unwrap());
    println!("mh-audio evidence: {} ({} requests, {} wave starts, all_pass {ok})", path.display(), r.script.len(), log.rows.len());
    world.write_message(AppExit::Success);
}

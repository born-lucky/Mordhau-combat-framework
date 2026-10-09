//! Private native-data probe for the runtime's actual stereo mixer, not another game executable.
//! Link against the reviewed runtime's mh_audio/mh_assets/mh_pak/mh_ui rlibs.
//! Set MORDHAU_LOCAL_DATA and MH_AUDIO_REPLACEMENTS; pass an evidence output directory.
use mh_audio::{AudioListener, AudioState, channel_map, mixer::{Pcm, Voice, VoiceParams}};
use std::{path::Path, sync::Arc, io::Write};

fn render(pcm: &Pcm, params: VoiceParams) -> (Vec<f32>, [f64; 2]) {
    let mut voice = Voice::new(pcm.clone(), params, None);
    let frames = (pcm.seconds() * 48000.0).ceil() as usize + 2048;
    let mut out = vec![0.0; frames * 2];
    voice.mix_stereo(&mut out);
    let mut energy = [0.0; 2];
    for frame in out.chunks_exact(2) {
        for c in 0..2 { energy[c] += (frame[c] as f64).powi(2); }
    }
    (out, energy)
}

fn wav(path: &Path, pcm: &[f32]) {
    let bytes = (pcm.len() * 2) as u32;
    let mut out = std::fs::File::create(path).unwrap();
    out.write_all(b"RIFF").unwrap(); out.write_all(&(bytes + 36).to_le_bytes()).unwrap();
    out.write_all(b"WAVEfmt ").unwrap(); out.write_all(&16u32.to_le_bytes()).unwrap();
    out.write_all(&1u16.to_le_bytes()).unwrap(); out.write_all(&2u16.to_le_bytes()).unwrap();
    out.write_all(&48000u32.to_le_bytes()).unwrap(); out.write_all(&192000u32.to_le_bytes()).unwrap();
    out.write_all(&4u16.to_le_bytes()).unwrap(); out.write_all(&16u16.to_le_bytes()).unwrap();
    out.write_all(b"data").unwrap(); out.write_all(&bytes.to_le_bytes()).unwrap();
    for &v in pcm { out.write_all(&((v * 32768.0).round().clamp(-32768.0, 32767.0) as i16).to_le_bytes()).unwrap(); }
}

fn main() {
    let folder = std::env::args().nth(1).expect("evidence output directory");
    let folder = Path::new(&folder); std::fs::create_dir_all(folder).unwrap();
    let paks = mh_ui::Paks(Arc::new(mh_pak::Vfs::mount_default().expect("owner game paks")));
    let mut state = AudioState::new(&paks);
    let path = "Mordhau/Content/Mordhau/Audio/Cues/Weapons/Wooshes/SC_Woosh_BladedMassive";
    let cue = state.cue(path).expect("native whoosh cue").clone();
    let att = cue.attenuation.expect("native whoosh attenuation");
    assert!(att.spatialize && att.attenuate, "Native cue is not spatialized/attenuated");
    let wave = &cue.waves[0];
    assert_eq!(state.sample_source(path, wave), "original");
    let pcm = state.cue_pcm(path, wave).expect("native wave PCM");
    assert_eq!(*pcm.samples, *state.wave_pcm(wave).unwrap().samples, "Withdrawn bank still substitutes audio");
    let listener = AudioListener::default();
    let lateral_m = (att.omni_radius_cm as f32 / 100.0 + 5.0).max(5.0);
    let mut energies = vec![];
    for (name, position, lis) in [
        ("left", [-lateral_m, 0.0, 0.0], listener),
        ("right", [lateral_m, 0.0, 0.0], listener),
        ("left-camera-turned", [-lateral_m, 0.0, 0.0], AudioListener { fwd: [0.0, 0.0, 1.0].into(), right: [-1.0, 0.0, 0.0].into(), ..listener }),
        ("near", [0.0, 0.0, -1.0], listener),
        ("far", [0.0, 0.0, -100.0], listener),
    ] {
        let position = position.into();
        let (map, az) = channel_map(Some(&att), pcm.channels, position, &lis);
        let distance = position.distance(lis.pos) as f64 * 100.0;
        let gain = mh_assets::sound_cue::gain(Some(&att), distance);
        let (mixed, energy) = render(&pcm, VoiceParams { gain: gain as f32, map, ..Default::default() });
        wav(&folder.join(format!("{name}.wav")), &mixed);
        println!("{name}: azimuth={az:.3}, distance_m={:.3}, gain={gain:.6}, L_energy={:.6}, R_energy={:.6}", distance / 100.0, energy[0], energy[1]);
        energies.push(energy);
    }
    assert!(energies[0][0] > energies[0][1] * 2.0, "Left source does not favor left speaker");
    assert!(energies[1][1] > energies[1][0] * 2.0, "Right source does not favor right speaker");
    assert!(energies[2][1] > energies[2][0] * 2.0, "Camera rotation does not rotate the stereo field");
    assert!(energies[3].iter().sum::<f64>() > energies[4].iter().sum::<f64>() * 2.0, "Distance does not attenuate playback");
    // A deliberate nonspatial UI cue must remain centered under the same mixer.
    let (map, _) = channel_map(None, pcm.channels, [5.0, 0.0, 0.0].into(), &listener);
    let (_, ui) = render(&pcm, VoiceParams { map, ..Default::default() });
    assert!((ui[0] - ui[1]).abs() < 1e-5, "Nonspatial cue unexpectedly pans");
    println!("ENG_OK: actual native PCM routes left/right, rotates with listener and attenuates with distance. Stereo mixer evidence only; no full vanilla audio parity claim.");
}

//! mh-audio: Mordhau's SoundCues in Bevy. A `PlayCue` (cue package + Bevy world position / velocity + cue params)
//! is parsed like UE's ParseNodes (mh-assets sound_cue::evaluate_ctx: Random with its HasBeenUsed state per cue,
//! Modulator, Switch, Delay, Concatenator, Enveloper, DistanceCrossFade at the listener distance, ...); each resulting
//! wave is scheduled after its delay, then started with the attenuation gain at the listener distance (the Play's
//! Attenuation node settings, else the cue's; sound_cue::gain = AttenuationEval) and the Doppler node's pitch
//! (sound_cue::doppler_pitch). Every start is an `AudioStarted` message and a row of the `AudioLog` (the evidence).
//!
//! Output: every start becomes a `mixer::Voice` (the wave's Ogg decoded once, then the shipped audio mixer's source
//! path: pitch resampling, volume ramps, envelope, clamps; rvas in mixer.rs). Attenuation and doppler are updated
//! every frame from the listener. With bevy's `AudioPlugin` present and a primary window (or MH_AUDIO=device) the
//! voice is a `bevy_audio` source (`CueVoice`, a `Decodable` asset) played by rodio on the device; otherwise
//! (headless, offscreen, no plugin, MH_AUDIO=offline) the voices are rendered by `OfflineMix` each frame instead
//! (optionally captured), so a run never needs a device and evidence runs stay silent. With the plugin but no
//! device bevy_audio only warns and nothing is pulled (no failure).
//!
//! Which cue a combat event plays comes from the weapon Blueprint's CDO chain (`WeaponSounds`).

pub mod ambient;
pub mod game;
pub mod mixer;
pub mod occlusion;
pub mod reverb;
pub mod routing;
pub mod sources;
pub mod spatial;
pub mod zones;

use bevy::prelude::*;
use mh_assets::sound_cue::{self, CueState, EvalCtx, SoundCue};
use mh_ui::Paks;
use mixer::{Pcm, Voice, VoiceParams};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// load a SoundCue, its routing and every wave it can play ahead of its first PlayCue, so the first play does not
/// read / decode on the frame it is heard (first-person r1: the 262 ms first-hit hitch; loading glue, no game rule)
#[derive(Message, Clone, Debug)]
pub struct PrewarmCue(pub String);

fn prewarm(mut ev: MessageReader<PrewarmCue>, st: Option<NonSendMut<AudioState>>) {
    let Some(mut st) = st else {
        ev.clear();
        return;
    };
    for PrewarmCue(c) in ev.read() {
        st.prewarm(c);
    }
}

/// play a SoundCue at a Bevy world position (metres, Y up) with a velocity (m/s, for doppler)
#[derive(Message, Clone, Debug)]
pub struct PlayCue {
    pub cue: String,
    pub pos: Vec3,
    pub vel: Vec3,
    /// Switch / Branch / ModulatorContinuous parameters
    pub params: HashMap<String, f64>,
    /// the playing component's VolumeMultiplier / PitchMultiplier (UAudioComponent; 1 for one-shots)
    pub volume: f64,
    pub pitch: f64,
    /// the component's attenuation (override or asset), replacing the sound's own
    pub attenuation: Option<sound_cue::Attenuation>,
    /// a 2D sound (UGameplayStatics::PlaySound2D, widget sounds): no attenuation, not spatialized
    pub two_d: bool,
}

impl Default for PlayCue {
    fn default() -> Self {
        PlayCue { cue: String::new(), pos: Vec3::ZERO, vel: Vec3::ZERO, params: HashMap::new(), volume: 1.0, pitch: 1.0, attenuation: None, two_d: false }
    }
}

/// the listener (the view camera): position (m), velocity (m/s), and its forward / right unit vectors (Bevy world;
/// the defaults are an unrotated Bevy camera: forward -Z, right +X)
#[derive(Resource, Clone, Copy, Debug)]
pub struct AudioListener {
    pub pos: Vec3,
    pub vel: Vec3,
    pub fwd: Vec3,
    pub right: Vec3,
}

impl Default for AudioListener {
    fn default() -> Self {
        AudioListener { pos: Vec3::ZERO, vel: Vec3::ZERO, fwd: Vec3::NEG_Z, right: Vec3::X }
    }
}

impl AudioListener {
    /// position, orientation and velocity (from the previous position over dt) from a camera transform
    pub fn follow(&mut self, t: &GlobalTransform, dt: f32) {
        let p = t.translation();
        self.vel = (p - self.pos) / dt.max(1e-4);
        self.pos = p;
        self.fwd = t.forward().as_vec3();
        self.right = t.right().as_vec3();
    }
}

/// one wave started: what an output backend plays (linear volume including attenuation, pitch including doppler)
#[derive(Message, Clone, Debug)]
pub struct AudioStarted {
    pub cue: String,
    pub wave: String,
    pub pos: Vec3,
    pub volume: f64,
    pub pitch: f64,
    pub looping: bool,
    pub loop_count: Option<i64>,
}

#[derive(Clone, Debug)]
pub struct LogRow {
    /// app time (s) of the start
    pub t: f64,
    pub cue: String,
    pub wave: String,
    pub delay: f64,
    pub distance_m: f64,
    /// cue x node volume x envelope (before attenuation)
    pub volume: f64,
    pub gain: f64,
    pub pitch: f64,
    pub doppler: f64,
    pub looping: bool,
    /// the sound class (empty = default) whose effective volume / pitch are in `volume` / `pitch`
    pub class: String,
}

impl LogRow {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({"t": self.t, "cue": self.cue, "wave": self.wave, "delay": self.delay, "distance_m": self.distance_m,
            "volume": self.volume, "gain": self.gain, "out_volume": self.volume * self.gain, "pitch": self.pitch,
            "doppler": self.doppler, "looping": self.looping, "class": self.class})
    }
}

/// evidence: every wave start, cues that did not load, and cues with nodes the evaluator does not know
#[derive(Resource, Clone, Debug, Default)]
pub struct AudioLog {
    pub rows: Vec<LogRow>,
    /// sounds concurrency kept from playing: (cue, reason)
    pub rejected: Vec<(String, String)>,
    /// sounds stopped by concurrency (voice stealing): cue
    pub stolen: Vec<String>,
    pub missing: Vec<String>,
    pub inexact: Vec<(String, Vec<String>)>,
}

pub struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<PlayCue>()
            .add_message::<PrewarmCue>()
            .add_message::<mh_ui::UiSound>()
            .add_message::<AudioStarted>()
            .init_resource::<AudioListener>()
            .init_resource::<AudioLog>()
            .init_resource::<OfflineMix>()
            .init_resource::<ListenerAmbient>()
            .init_resource::<ReverbState>()
            .add_systems(Startup, setup)
            .add_systems(Update, (prewarm, map_sounds, random_spawners, ui_sounds, start, emit, update_voices).chain());
    }

    /// after every plugin's build: device output when bevy's AudioPlugin is in the app and the app has a primary
    /// window (the playable game), else the offline mixer (headless / offscreen evidence runs stay silent);
    /// env MH_AUDIO=device / offline overrides
    fn finish(&self, app: &mut App) {
        let bevy_audio = app.is_plugin_added::<bevy::audio::AudioPlugin>();
        let windowed = app.world_mut().query_filtered::<(), With<bevy::window::PrimaryWindow>>().iter(app.world()).next().is_some();
        let device = bevy_audio
            && match std::env::var("MH_AUDIO").as_deref() {
                Ok("device") => true,
                Ok("offline") => false,
                _ => windowed,
            };
        if device {
            use bevy::audio::AddAudioSource;
            app.add_audio_source::<CueVoice>();
            app.add_audio_source::<reverb::BusVoice>();
            app.add_systems(Startup, spawn_bus);
        }
        app.insert_resource(AudioOutput { device });
    }
}

/// where voices go: `device` = bevy_audio (rodio) players, else `OfflineMix`
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct AudioOutput {
    pub device: bool,
}

/// shared between the game thread and the voice (device thread or offline mixer)
#[derive(Debug, Default)]
pub struct VoiceStatus {
    /// output frames rendered (at mixer::MIXER_RATE)
    pub rendered: AtomicU64,
    pub finished: AtomicBool,
    /// the device pulled this voice (rodio created its decoder)
    pub pulled: AtomicBool,
}

/// a playing wave as the game thread sees it: live parameters in, status out
#[derive(Clone)]
pub struct VoiceHandle {
    pub cue: String,
    pub wave: String,
    pub pos: Vec3,
    pub vel: Vec3,
    pub ctl: Arc<Mutex<VoiceParams>>,
    pub status: Arc<VoiceStatus>,
    att: Option<sound_cue::Attenuation>,
    doppler: Option<sound_cue::Doppler>,
    play: sound_cue::Play,
    channels: u16,
    /// the PlayCue start this wave belongs to (concurrency member)
    pub inst: u64,
    pub class: String,
    /// the sound's playback time (s), advanced by the frame time (game side)
    pub playback: f64,
    /// the azimuth the current channel map was computed at (recomputed when it moves > 0.01 degrees)
    last_az: Option<f32>,
    /// ambient zone state (zones.rs, FActiveSound::HandleInteriorVolumes 0x2e2fde0)
    zone: zones::SoundZone,
    /// occlusion state (occlusion.rs, FActiveSound::CheckOcclusion 0x2e23880)
    occ: occlusion::SoundOcclusion,
}

/// the channel map of a source at `pos` for the listener (spatial.rs)
pub fn channel_map(att: Option<&sound_cue::Attenuation>, channels: u16, pos: Vec3, lis: &AudioListener) -> ([[f32; 2]; 2], f32) {
    let d = pos - lis.pos;
    let dist_cm = d.length() * 100.0;
    let dir = d.normalize_or_zero();
    let az = spatial::absolute_azimuth(lis.fwd.to_array(), lis.right.to_array(), dir.to_array());
    let spatialize = att.is_some_and(|a| a.spatialize);
    let omni = att.map_or(0.0, |a| spatial::normalized_omni(a.omni_radius_cm as f32, dist_cm));
    let spread = att.map_or(200.0, |a| a.stereo_spread_cm as f32);
    (spatial::channel_map(channels, spatialize, az, omni, spread, dist_cm), az)
}

/// a voice as a bevy_audio source: rodio pulls `VoiceSource` (stereo f32 at 48 kHz) on its device thread
#[derive(Asset, TypePath)]
pub struct CueVoice {
    bus: Arc<reverb::Bus>,
    voice: Mutex<Option<Voice>>,
    ctl: Arc<Mutex<VoiceParams>>,
    status: Arc<VoiceStatus>,
}

pub struct VoiceSource {
    bus: Arc<reverb::Bus>,
    /// the output frame this voice's next block starts at (the bus's playback position at its first pull)
    at: Option<u64>,
    voice: Option<Voice>,
    ctl: Arc<Mutex<VoiceParams>>,
    status: Arc<VoiceStatus>,
    buf: Vec<f32>,
    pos: usize,
}

impl VoiceSource {
    /// one mixer block: live params in, stereo out
    fn fill(&mut self) -> bool {
        let Some(v) = self.voice.as_mut() else { return false };
        if let Ok(p) = self.ctl.lock() {
            v.params = *p;
        }
        self.buf.clear();
        self.buf.resize(mixer::BLOCK * 2, 0.0);
        let mut send = vec![0.0f32; mixer::BLOCK * 2];
        let n = v.mix_stereo_send(&mut self.buf, Some(&mut send));
        self.buf.truncate(n * 2);
        let at = *self.at.get_or_insert_with(|| self.bus.pos.load(Ordering::Relaxed));
        if v.params.reverb_send > 0.0 {
            self.bus.add_send(at, &send[..n * 2]);
        }
        self.at = Some(at + n as u64);
        self.pos = 0;
        self.status.rendered.store(v.rendered, Ordering::Relaxed);
        if v.finished {
            self.status.finished.store(true, Ordering::Relaxed);
        }
        n > 0
    }
}

impl Iterator for VoiceSource {
    type Item = rodio::Sample;
    fn next(&mut self) -> Option<f32> {
        if self.pos >= self.buf.len() && !self.fill() {
            self.status.finished.store(true, Ordering::Relaxed);
            return None;
        }
        let s = self.buf[self.pos];
        self.pos += 1;
        Some(s)
    }
}

impl rodio::Source for VoiceSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> rodio::ChannelCount {
        rodio::ChannelCount::new(2).unwrap()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        rodio::SampleRate::new(mixer::MIXER_RATE).unwrap()
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
    fn try_seek(&mut self, _pos: std::time::Duration) -> Result<(), rodio::source::SeekError> {
        Err(rodio::source::SeekError::NotSupported { underlying_source: "mh_audio::VoiceSource" })
    }
}

impl bevy::audio::Decodable for CueVoice {
    type Decoder = VoiceSource;
    fn decoder(&self) -> VoiceSource {
        self.status.pulled.store(true, Ordering::Relaxed);
        VoiceSource { bus: self.bus.clone(), at: None, voice: self.voice.lock().ok().and_then(|mut v| v.take()), ctl: self.ctl.clone(), status: self.status.clone(), buf: vec![], pos: 0 }
    }
}

/// the no-device fallback: voices rendered on the game thread, `elapsed x 48 kHz` frames per update; `capture`
/// keeps the stereo output (evidence)
#[derive(Resource, Default)]
pub struct OfflineMix {
    pub voices: Vec<(Voice, Arc<Mutex<VoiceParams>>, Arc<VoiceStatus>)>,
    pub capture: Option<Vec<f32>>,
    /// output frames rendered so far (the capture's length / 2)
    pub frames: u64,
    /// per voice start: (wave, output frame it started at)
    pub starts: Vec<(String, u64)>,
    /// the master reverb submix (also the device path's bus)
    pub bus: Arc<reverb::Bus>,
}

impl OfflineMix {
    pub fn render(&mut self, frames: usize) {
        let mut out = vec![0.0f32; frames * 2];
        let mut send = vec![0.0f32; frames * 2];
        for (v, ctl, st) in &mut self.voices {
            if let Ok(p) = ctl.lock() {
                v.params = *p;
            }
            v.mix_stereo_send(&mut out, Some(&mut send));
            st.rendered.store(v.rendered, Ordering::Relaxed);
            st.finished.store(v.finished, Ordering::Relaxed);
        }
        self.voices.retain(|(v, _, _)| !v.finished);
        if let Ok(mut r) = self.bus.reverb.lock() {
            for (i, o) in send.chunks(mixer::BLOCK * 2).zip(out.chunks_mut(mixer::BLOCK * 2)) {
                r.process(i, o);
            }
        }
        if let Some(c) = self.capture.as_mut() {
            c.extend_from_slice(&out);
        }
        self.frames += frames as u64;
    }
}

/// the cue fields of a weapon Blueprint's CDO chain (the names are the Blueprint's; which combat event plays which is
/// read from the names: UNCONFIRMED against the code that plays them)
#[derive(Clone, Debug, Default)]
pub struct WeaponSounds {
    pub strike_hit: String,
    pub stab_hit: String,
    pub blocked: String,
    pub was_blocked: String,
    pub hit_cancel: String,
    pub strike_woosh: String,
    pub stab_woosh: String,
    pub environment_hit: String,
    pub equip: String,
}

impl WeaponSounds {
    pub fn read(rd: &mh_pak::Reader, weapon_bp: &str) -> WeaponSounds {
        let d = rd.defaults(weapon_bp);
        let g = |k: &str| mh_assets::material::ue_pkg_path(d.get(k).unwrap_or(&serde_json::Value::Null));
        WeaponSounds {
            strike_hit: g("StrikeHitSound"),
            stab_hit: g("StabHitSound"),
            blocked: g("BlockedSound"),
            was_blocked: g("WasBlockedSound"),
            hit_cancel: g("HitCancelSound"),
            strike_woosh: g("StrikeWooshSound"),
            stab_woosh: g("StabWooshSound"),
            environment_hit: g("EnvironmentHitSound"),
            equip: g("EquipSound"),
        }
    }

    /// the cue a mh-sim drain() event plays: hit -> Strike/StabHitSound (by the event's "stab" flag), parry /
    /// active_parry / chamber -> BlockedSound, clash -> HitCancelSound
    pub fn for_event(&self, ev: &serde_json::Value) -> Option<&str> {
        let k = ev.get("kind").and_then(|k| k.as_str())?;
        let s = match k {
            "hit" => {
                if ev.get("stab").and_then(|s| s.as_bool()).unwrap_or(false) {
                    &self.stab_hit
                } else {
                    &self.strike_hit
                }
            }
            "parry" | "active_parry" | "chamber" => &self.blocked,
            "clash" => &self.hit_cancel,
            _ => return None,
        };
        (!s.is_empty()).then_some(s.as_str())
    }
}

/// a wave scheduled after its delay
struct Pending {
    t: f64,
    req: PlayCue,
    play: sound_cue::Play,
    inst: u64,
    class: String,
}

/// cue cache, Random-node state per cue, pending delayed starts (NonSend: mh-pak Reader)
pub struct AudioState {
    pub rd: mh_pak::Reader,
    src: mh_assets::pak_source::PakSource,
    cues: HashMap<String, Option<SoundCue>>,
    states: HashMap<String, CueState>,
    rng: mh_assets::particles::RandomStream,
    pending: Vec<Pending>,
    /// sound classes + passive mixes
    pub classes: routing::Classes,
    routes: HashMap<String, Option<mh_assets::sound_class::Routing>>,
    /// concurrency groups: key -> active sounds
    pub groups: HashMap<String, Vec<routing::Member>>,
    next_inst: u64,
    /// decoded waves (None = no Ogg payload / undecodable)
    pcm: HashMap<String, Option<Pcm>>,
    /// playing voices (attenuation / doppler updated every frame)
    pub voices: Vec<VoiceHandle>,
}

impl AudioState {
    pub fn new(p: &Paks) -> AudioState {
        AudioState {
            rd: mh_pak::Reader::new(p.0.clone()),
            src: mh_assets::pak_source::PakSource::new(p.0.clone()),
            cues: HashMap::new(),
            states: HashMap::new(),
            rng: mh_assets::particles::RandomStream(0x00a0d10),
            pending: vec![],
            classes: routing::Classes::new(mh_assets::sound_class::ClassTree::load(&mh_pak::Reader::new(p.0.clone()))),
            routes: HashMap::new(),
            groups: HashMap::new(),
            next_inst: 1,
            pcm: HashMap::new(),
            voices: vec![],
        }
    }

    /// a cue's routing (class / concurrency / submix), cached; its class joins the class tree
    pub fn routing(&mut self, cue: &str) -> Option<mh_assets::sound_class::Routing> {
        if !self.routes.contains_key(cue) {
            let r = mh_assets::sound_class::routing(&self.rd, cue);
            if let Some(r) = &r {
                let rd = mh_pak::Reader::new(self.rd.vfs.clone());
                self.classes.tree.add(&rd, &r.class);
            }
            self.routes.insert(cue.to_string(), r);
        }
        self.routes[cue].clone()
    }

    /// stop every wave of a sound instance (concurrency voice stealing)
    fn stop_inst(&mut self, inst: u64) {
        self.pending.retain(|q| q.inst != inst);
        for v in &self.voices {
            if v.inst == inst {
                if let Ok(mut c) = v.ctl.lock() {
                    c.stop = true;
                }
            }
        }
        for g in self.groups.values_mut() {
            g.retain(|m| m.inst != inst);
        }
    }

    /// delayed starts not yet due
    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    /// the cue, its routing and every wave of its graph decoded (cached; PrewarmCue)
    pub fn prewarm(&mut self, cue: &str) {
        if cue.is_empty() {
            return;
        }
        let waves = self.cue(cue).map(|c| c.waves.clone()).unwrap_or_default();
        let _ = self.routing(cue);
        for w in waves {
            let _ = self.wave_pcm(&w);
        }
    }

    /// a wave decoded to PCM (cached)
    pub fn wave_pcm(&mut self, wave: &str) -> Option<Pcm> {
        if !self.pcm.contains_key(wave) {
            let p = self.wave_ogg(wave).and_then(|b| mixer::decode_ogg(&b).map_err(|e| warn!("mh-audio: {wave}: {e}")).ok());
            self.pcm.insert(wave.to_string(), p);
        }
        self.pcm[wave].clone()
    }

    /// the Ogg Vorbis payload of a wave package (for an output backend)
    pub fn wave_ogg(&self, wave: &str) -> Option<Vec<u8>> {
        mh_assets::sound::ogg(&self.src, wave).ok()
    }

    pub fn cue(&mut self, path: &str) -> Option<&SoundCue> {
        if !self.cues.contains_key(path) {
            let c = sound_cue::cue(&self.rd, path);
            self.cues.insert(path.to_string(), c);
        }
        self.cues[path].as_ref()
    }
}

fn setup(world: &mut World) {
    let Some(p) = Paks::get_or_mount(world) else {
        warn!("mh-audio: no paks; sounds disabled");
        return;
    };
    world.insert_non_send(AudioState::new(&p));
}

/// a map's placed sounds (ambient::map_sounds); inserting / replacing it starts every auto-activated one
#[derive(Resource, Clone, Debug, Default)]
pub struct MapSounds(pub Vec<ambient::MapSound>);

fn map_sounds(m: Option<Res<MapSounds>>, mut out: MessageWriter<PlayCue>) {
    let Some(m) = m.filter(|m| m.is_changed()) else { return };
    for s in m.0.iter().filter(|s| s.auto_activate) {
        let p = mh_assets::coords::pos(s.pos_ue.map(|x| x as f32));
        out.write(PlayCue { cue: s.sound.clone(), pos: Vec3::from_array(p), volume: s.volume, pitch: s.pitch, attenuation: s.attenuation.clone(), ..default() });
    }
}

/// the widgets' sounds (mh_ui::UiSound: hover / click / PlaySound2D) play 2D at the listener, in their cue's class
fn ui_sounds(mut ui: MessageReader<mh_ui::UiSound>, lis: Res<AudioListener>, mut out: MessageWriter<PlayCue>) {
    for s in ui.read() {
        out.write(PlayCue { cue: s.cue.clone(), pos: lis.pos, two_d: true, ..default() });
    }
}

/// the reverb bus as a device source (plays for the app's lifetime)
fn spawn_bus(mut commands: Commands, offline: Res<OfflineMix>, mut assets: ResMut<Assets<reverb::BusVoice>>) {
    let h = assets.add(reverb::BusVoice(offline.bus.clone()));
    commands.spawn(bevy::audio::AudioPlayer(h));
}

/// the reverb parameters' game side: FAudioEffectsManager's state, the listener volume they were last set for, and
/// the ReverbEffect assets read so far
#[derive(Resource, Default)]
pub struct ReverbState {
    pub mgr: reverb::EffectsManager,
    last_volume: Option<u32>,
    fx: HashMap<String, Option<reverb::ReverbEffect>>,
}

impl ReverbState {
    fn effect(&mut self, rd: &mh_pak::Reader, pkg: &str) -> Option<reverb::ReverbEffect> {
        self.fx
            .entry(pkg.to_string())
            .or_insert_with(|| {
                let ex = rd.read(pkg)?;
                let e = ex.iter().find(|e| e.get("Type").and_then(|t| t.as_str()) == Some("ReverbEffect"))?;
                Some(reverb::ReverbEffect::from_asset(e.get("Properties").unwrap_or(&serde_json::Value::Null)))
            })
            .clone()
    }

    /// UpdateAudioVolumeEffects 0x2f01208..0x2f01384: when the listener's volume changes, its FReverbSettings (the
    /// world's defaults outside every volume) go to SetReverbSettings; every frame the effect interpolates and
    /// FSubmixEffectReverb::SetParameters runs on it
    fn update(&mut self, rd: &mh_pak::Reader, vols: Option<&MapVolumes>, zone: u32, now: f64, bus: &reverb::Bus) {
        if self.last_volume != Some(zone) {
            self.last_volume = Some(zone);
            let s = match vols.and_then(|v| v.0.get((zone as usize).wrapping_sub(1))) {
                Some(v) => reverb::ReverbSettings {
                    apply: v.apply_reverb,
                    effect: (!v.reverb.is_empty()).then(|| self.effect(rd, &v.reverb).map(|e| (v.reverb.clone(), e))).flatten(),
                    volume: v.reverb_volume as f32,
                    fade_time: v.reverb_fade as f32,
                },
                None => reverb::ReverbSettings::default(),
            };
            self.mgr.set(&s, now);
        }
        let fx = self.mgr.update(now);
        let (s, wet) = fx.plate_settings();
        if let Ok(mut r) = bus.reverb.lock() {
            r.set(s, wet);
        }
    }
}

/// a map's AudioVolumes (ambient::audio_volumes): their AmbientZoneSettings apply to every sound (zones.rs)
#[derive(Resource, Clone, Debug, Default)]
pub struct MapVolumes(pub Vec<ambient::AudioVol>);

/// the listener's ambient zone (zones::ListenerZone, FAudioDevice::UpdateAudioVolumeEffects 0x2f00d80)
#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct ListenerAmbient(pub zones::ListenerZone);

/// a sound's ambient zone volume multiplier and the lowest of its cutoffs: class LowPassFilterFrequency, ambient
/// zone, attenuation LPF (FMixerSource takes the minimum; UNCONFIRMED: the exe's FMixerSource::UpdateEffects was
/// not read, the minimum is the engine's documented rule)
fn zone_and_lpf(z: &mut zones::SoundZone, vols: Option<&MapVolumes>, lis: &zones::ListenerZone, att: Option<&sound_cue::Attenuation>, pos: Vec3, class_lpf: f64, dist_cm: f64, now: f64) -> (f32, f32) {
    let (zv, zf) = match vols {
        Some(v) if !v.0.is_empty() => {
            let spatialized = att.is_some_and(|a| a.spatialize);
            z.apply(lis, zones::zone_at(&v.0, to_ue(pos)), spatialized, now)
        }
        _ => (1.0, zones::MAX_LPF),
    };
    let af = att.map_or(zones::MAX_LPF, |a| sound_cue::attenuation_lpf(a, dist_cm) as f32);
    (zv, zf.min(class_lpf as f32).min(af))
}

/// the occlusion step of a sound (occlusion.rs): trace listener -> sound when a tracer is present
fn occlude(o: &mut occlusion::SoundOcclusion, tracer: Option<&occlusion::OcclusionTracer>, att: Option<&sound_cue::Attenuation>, audible: bool, pos: Vec3, lis: &AudioListener) -> (f32, f32) {
    match tracer {
        Some(t) => o.update(att, audible, |ch| (t.0)(to_ue(lis.pos), to_ue(pos), ch)),
        None => (1.0, 20000.0),
    }
}

/// a map's BP_AmbientRandomAudioSpawner actors (ambient::random_spawners); inserting / replacing it is their BeginPlay
#[derive(Resource, Clone, Debug, Default)]
pub struct MapSpawners(pub Vec<ambient::RandomSpawner>);

/// BP_AmbientRandomAudioSpawner's event graph (bytecode of ExecuteUbergraph_BP_AmbientRandomAudioSpawner, read with
/// scripts/kismet): BeginPlay (statement 741 -> 15, a DoOnce) -> t = RandomFloatInRange(Time_Min, Time_Max),
/// K2_SetTimer(Sound_Trig, t, looping); Sound_Trig (194): z = RandomFloatInRange(0, MaxHeight), y =
/// RandomFloatInRange(-Distance, Distance), x = the same, SpawnSoundAtLocation(Sound, actor location + (x, y, z),
/// volume 1, pitch 1), then Retrigger (736 -> 61): a new t and K2_SetTimer again. FMath::FRand's generator is the
/// process-global one in UE; here mh-audio's stream (UNCONFIRMED sequence).
fn random_spawners(m: Option<Res<MapSpawners>>, time: Res<Time>, st: Option<NonSendMut<AudioState>>, mut next: Local<Vec<f64>>, mut out: MessageWriter<PlayCue>) {
    let (Some(m), Some(mut st)) = (m, st) else { return };
    let now = time.elapsed_secs_f64();
    let mut rand_range = |lo: f64, hi: f64| lo + (hi - lo) * st.rng.fraction() as f64;
    if m.is_changed() || next.len() != m.0.len() {
        *next = m.0.iter().map(|s| now + rand_range(s.time_min, s.time_max)).collect();
    }
    for (i, s) in m.0.iter().enumerate() {
        if now < next[i] || s.sound.is_empty() {
            continue;
        }
        let z = rand_range(0.0, s.max_height);
        let y = rand_range(-s.distance, s.distance);
        let x = rand_range(-s.distance, s.distance);
        let ue = [(s.pos_ue[0] + x) as f32, (s.pos_ue[1] + y) as f32, (s.pos_ue[2] + z) as f32];
        out.write(PlayCue { cue: s.sound.clone(), pos: Vec3::from_array(mh_assets::coords::pos(ue)), ..default() });
        next[i] = now + rand_range(s.time_min, s.time_max);
    }
}

/// Bevy world (m, Y up) -> UE (cm, Z up)
fn to_ue(v: Vec3) -> [f64; 3] {
    [v.x as f64 * 100.0, v.z as f64 * 100.0, v.y as f64 * 100.0]
}

fn start(st: Option<NonSendMut<AudioState>>, mut reqs: MessageReader<PlayCue>, lis: Res<AudioListener>, time: Res<Time>, mut log: ResMut<AudioLog>) {
    let Some(mut st) = st else { return };
    let now = time.elapsed_secs_f64();
    for r in reqs.read() {
        let dist_cm = r.pos.distance(lis.pos) as f64 * 100.0;
        let Some(c) = st.cue(&r.cue).cloned() else {
            if !log.missing.contains(&r.cue) {
                log.missing.push(r.cue.clone());
            }
            continue;
        };
        if !c.exact() && !log.inexact.iter().any(|(p, _)| *p == r.cue) {
            log.inexact.push((r.cue.clone(), c.unknown.clone()));
        }
        let st = &mut *st;
        // concurrency (routing.rs): every group of the sound must admit it
        let route = st.routing(&r.cue);
        let mut stop = vec![];
        let mut rejected = None;
        for cc in route.iter().flat_map(|x| x.concurrency.iter()) {
            let key = routing::group_key(cc, &r.cue);
            let members = st.groups.get(&key).cloned().unwrap_or_default();
            match routing::evaluate(cc, &members, now, dist_cm) {
                routing::Verdict::Play { stop: s } => stop.extend(s),
                routing::Verdict::Reject(why) => {
                    rejected = Some(why);
                    break;
                }
            }
        }
        if let Some(why) = rejected {
            log.rejected.push((r.cue.clone(), why.to_string()));
            continue;
        }
        for i in stop {
            if let Some(v) = st.voices.iter().find(|v| v.inst == i) {
                log.stolen.push(v.cue.clone());
            }
            st.stop_inst(i);
        }
        let inst = st.next_inst;
        st.next_inst += 1;
        for cc in route.iter().flat_map(|x| x.concurrency.iter()) {
            st.groups.entry(routing::group_key(cc, &r.cue)).or_default().push(routing::Member { inst, start: now, dist_cm });
        }
        let class = route.map(|x| x.class).unwrap_or_default();
        let ctx = EvalCtx { params: r.params.clone(), distance_cm: Some(dist_cm) };
        let state = st.states.entry(r.cue.clone()).or_default();
        let rng = &mut st.rng;
        let plays = sound_cue::evaluate_ctx(&c, &ctx, &mut || rng.fraction() as f64, state);
        for p in plays {
            st.pending.push(Pending { t: now + p.delay, req: r.clone(), play: p, inst, class: class.clone() });
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn emit(
    mut commands: Commands,
    st: Option<NonSendMut<AudioState>>,
    lis: Res<AudioListener>,
    time: Res<Time>,
    output: Option<Res<AudioOutput>>,
    mut offline: ResMut<OfflineMix>,
    mut voices: Option<ResMut<Assets<CueVoice>>>,
    mut log: ResMut<AudioLog>,
    mut out: MessageWriter<AudioStarted>,
    vols: Option<Res<MapVolumes>>,
    amb: Res<ListenerAmbient>,
    tracer: Option<Res<occlusion::OcclusionTracer>>,
) {
    let Some(mut st) = st else { return };
    let now = time.elapsed_secs_f64();
    let (due, keep): (Vec<_>, Vec<_>) = st.pending.drain(..).partition(|q| q.t <= now);
    st.pending = keep;
    let device = output.is_some_and(|o| o.device);
    for Pending { req: r, play: p, inst, class, .. } in due {
        let cp = st.classes.props(&class);
        let dist_cm = r.pos.distance(lis.pos) as f64 * 100.0;
        let att = if r.two_d { None } else { p.attenuation.clone().or_else(|| r.attenuation.clone()).or_else(|| st.cues.get(&r.cue).and_then(|c| c.as_ref()).and_then(|c| c.attenuation.clone())) };
        let gain = sound_cue::gain(att.as_ref(), dist_cm);
        let doppler = p.doppler.as_ref().map_or(1.0, |d| sound_cue::doppler_pitch(d, to_ue(r.pos), to_ue(r.vel), to_ue(lis.pos), to_ue(lis.vel)));
        let (ev, ep) = p.envelope_at(0.0);
        // the component multipliers ride on the wave's own volume / pitch
        let mut p = p;
        p.volume *= r.volume;
        p.pitch *= r.pitch;
        let volume = p.volume * ev * cp.volume;
        let pitch = p.pitch * ep * doppler * cp.pitch;
        log.rows.push(LogRow { t: now, cue: r.cue.clone(), wave: p.wave.clone(), delay: p.delay, distance_m: dist_cm / 100.0, volume, gain, pitch, doppler, looping: p.looping, class: class.clone() });
        out.write(AudioStarted { cue: r.cue.clone(), wave: p.wave.clone(), pos: r.pos, volume: volume * gain, pitch, looping: p.looping, loop_count: p.loop_count });
        // the voice: envelope / attenuation / doppler / channel map from the game side (update_voices every frame);
        // looping = forever, a Looping node's count
        let Some(pcm) = st.wave_pcm(&p.wave) else { continue };
        let (map, az) = channel_map(att.as_ref(), pcm.channels, r.pos, &lis);
        let mut zone = zones::SoundZone::default();
        let (zv, lpf) = zone_and_lpf(&mut zone, vols.as_deref(), &amb.0, att.as_ref(), r.pos, cp.lpf, dist_cm, now);
        let mut occ = occlusion::SoundOcclusion::default();
        let (ov, of) = occlude(&mut occ, tracer.as_deref(), att.as_ref(), volume * gain > 0.0, r.pos, &lis);
        let lpf = lpf.min(of);
        let params = VoiceParams { volume: (p.volume * cp.volume) as f32, gain: gain as f32 * zv * ov, pitch: (p.pitch * cp.pitch) as f32, doppler: doppler as f32, env: (ev as f32, ep as f32), map, lpf, reverb_send: reverb::send_level(att.as_ref(), cp.reverb, cp.is_music, dist_cm), stop: false };
        let loops = if p.looping { Some(-1) } else { p.loop_count };
        let channels = pcm.channels;
        let voice = Voice::new(pcm, params, loops);
        let ctl = Arc::new(Mutex::new(params));
        let status = Arc::new(VoiceStatus::default());
        match voices.as_mut().filter(|_| device) {
            Some(assets) => {
                let h = assets.add(CueVoice { bus: offline.bus.clone(), voice: Mutex::new(Some(voice)), ctl: ctl.clone(), status: status.clone() });
                commands.spawn((bevy::audio::AudioPlayer(h), bevy::audio::PlaybackSettings::DESPAWN));
            }
            None => {
                let at = offline.frames;
                offline.starts.push((p.wave.clone(), at));
                offline.voices.push((voice, ctl.clone(), status.clone()));
            }
        }
        st.voices.push(VoiceHandle { cue: r.cue.clone(), wave: p.wave.clone(), pos: r.pos, vel: r.vel, ctl, status, att, doppler: p.doppler.clone(), play: p.clone(), channels, playback: 0.0, last_az: Some(az), inst, class, zone, occ });
    }
}

/// every frame: attenuation and doppler from the listener into the playing voices; finished voices dropped; the
/// offline mixer advances by the frame's time
#[allow(clippy::too_many_arguments)]
fn update_voices(
    st: Option<NonSendMut<AudioState>>,
    lis: Res<AudioListener>,
    time: Res<Time>,
    output: Option<Res<AudioOutput>>,
    mut offline: ResMut<OfflineMix>,
    vols: Option<Res<MapVolumes>>,
    mut amb: ResMut<ListenerAmbient>,
    mut rv: ResMut<ReverbState>,
    tracer: Option<Res<occlusion::OcclusionTracer>>,
) {
    let now = time.elapsed_secs_f64();
    let mut zone = 0;
    if let Some(v) = vols.as_deref().filter(|v| !v.0.is_empty()) {
        let z = zones::zone_at(&v.0, to_ue(lis.pos));
        zone = z.id;
        amb.0.update(z, now);
    }
    if let Some(st) = st.as_ref() {
        rv.update(&st.rd, vols.as_deref(), zone, now, &offline.bus);
    }
    if let Some(mut st) = st {
        let dt = time.delta_secs();
        // passive mixes from the playing waves (volume x attenuation), then the classes' effective properties
        let playing: Vec<(String, f64)> = st.voices.iter().map(|v| (v.class.clone(), v.play.volume * v.ctl.lock().map_or(1.0, |c| c.gain as f64))).collect();
        st.classes.update(&playing, dt as f64);
        let st = &mut *st;
        for v in &mut st.voices {
            // the source travels with the velocity it was started with (stand-in for the actor UE attaches the
            // sound to; UNCONFIRMED for sounds fired at a fixed location)
            v.pos += v.vel * dt;
            // the ActiveSound's playback time advances by the frame time (UNCONFIRMED: not pitch-scaled; the wave
            // instance's own PlaybackTime += DeltaTime x Pitch, FActiveSound::UpdateWaveInstances 0x2e40e0e)
            v.playback += dt as f64;
            let dist_cm = v.pos.distance(lis.pos) as f64 * 100.0;
            let gain = sound_cue::gain(v.att.as_ref(), dist_cm);
            let doppler = v.doppler.as_ref().map_or(1.0, |d| sound_cue::doppler_pitch(d, to_ue(v.pos), to_ue(v.vel), to_ue(lis.pos), to_ue(lis.vel)));
            let (ev, ep) = v.play.envelope_at(v.playback);
            let (map, az) = channel_map(v.att.as_ref(), v.channels, v.pos, &lis);
            let remap = v.last_az.is_none_or(|a| (az - a).abs() > 0.01);
            if remap {
                v.last_az = Some(az);
            }
            let cp = st.classes.props(&v.class);
            let (zv, lpf) = zone_and_lpf(&mut v.zone, vols.as_deref(), &amb.0, v.att.as_ref(), v.pos, cp.lpf, dist_cm, now);
            let (ov, of) = occlude(&mut v.occ, tracer.as_deref(), v.att.as_ref(), v.play.volume * cp.volume * gain > 0.0, v.pos, &lis);
            let (zv, lpf) = (zv * ov, lpf.min(of));
            if let Ok(mut c) = v.ctl.lock() {
                c.volume = (v.play.volume * cp.volume) as f32;
                c.pitch = (v.play.pitch * cp.pitch) as f32;
                c.gain = gain as f32 * zv;
                c.lpf = lpf;
                c.reverb_send = reverb::send_level(v.att.as_ref(), cp.reverb, cp.is_music, dist_cm);
                c.doppler = doppler as f32;
                c.env = (ev as f32, ep as f32);
                if remap {
                    c.map = map;
                }
            }
        }
        st.voices.retain(|v| !v.status.finished.load(Ordering::Relaxed));
        // a sound leaves its concurrency groups when none of its waves plays or waits
        let live: Vec<u64> = st.voices.iter().map(|v| v.inst).chain(st.pending.iter().map(|q| q.inst)).collect();
        for g in st.groups.values_mut() {
            g.retain(|m| live.contains(&m.inst));
        }
    }
    if !output.is_some_and(|o| o.device) {
        // frames owed so far at 48 kHz (rounding carried, so the total tracks the elapsed time exactly)
        let want = (time.elapsed_secs_f64() * mixer::MIXER_RATE as f64) as u64;
        if want > offline.frames {
            let n = (want - offline.frames) as usize;
            offline.render(n);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BP_Longsword's cues resolve through the CDO chain, play waves of the cue, attenuate with distance, and the
    /// first wave's payload is Ogg
    #[test]
    fn longsword_sounds_and_attenuation() {
        let p = Paks(std::sync::Arc::new(mh_pak::Vfs::mount_default().expect("paks")));
        let mut st = AudioState::new(&p);
        let w = WeaponSounds::read(&st.rd, "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword");
        assert!(!w.strike_hit.is_empty() && !w.blocked.is_empty() && !w.hit_cancel.is_empty(), "{w:?}");
        let hit = w.for_event(&serde_json::json!({"kind": "hit"})).unwrap().to_string();
        let c = st.cue(&hit).cloned().expect("hit cue");
        assert!(!c.waves.is_empty());
        let near = sound_cue::gain(c.attenuation.as_ref(), 100.0);
        let far = sound_cue::gain(c.attenuation.as_ref(), 5000.0);
        assert!(near >= far, "{near} {far}");
        let mut s = CueState::default();
        let mut r = mh_assets::particles::RandomStream(1);
        let plays = sound_cue::evaluate_ctx(&c, &EvalCtx { distance_cm: Some(100.0), ..Default::default() }, &mut || r.fraction() as f64, &mut s);
        assert!(!plays.is_empty() && plays.iter().all(|p| c.waves.contains(&p.wave)));
        assert!(st.wave_ogg(&plays[0].wave).is_some_and(|b| b.starts_with(b"OggS")));
    }
}

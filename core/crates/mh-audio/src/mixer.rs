//! Engine-neutral software mixer voice: a port of the shipped UE 4.26 audio mixer's per-source path
//! (Mordhau-Win64-Shipping.exe, rvas from extract/split/symbols.tsv, read with scripts/ue_dis.py), so that what the
//! device plays (lib.rs, through bevy_audio) and what the evidence checks (decoded samples, no device) are the same
//! numbers.
//!
//! Which mixer: the paks' Engine/Config/Windows/WindowsEngine.ini [Audio] UseAudioMixer=true,
//! AudioMixerModuleName=AudioMixerXAudio2, PlatformHeadroomDB=-3; Mordhau/Config/DefaultEngine.ini
//! [/Script/WindowsTargetPlatform.WindowsTargetSettings] AudioSampleRate=48000, AudioCallbackBufferFrameSize=1024.
//!
//! Game side, once per game frame (FAudioDevice::Update 0x2f00270 -> FMixerSource::Update 0x2cdab00; the commands
//! reach the mixer at its next block; lib.rs writes `VoiceParams` every frame, the voice reads them per block):
//! - pitch = WaveInstance pitch (cue x node x envelope x doppler), FAudioDevice::ClampPitch 0x2eeef70 to
//!   [GlobalMinPitch, GlobalMaxPitch] = UAudioSettings ctor 0x2f07050 0.4 / 2.0 (FAudioDevice::Init 0x2ef6ff0:
//!   max(x, 0.0001)), then x MixerBuffer SampleRate / device SampleRate;
//! - volume (FMixerSource::UpdateVolume 0x2cdcac0) = GetVolume x GetDynamicVolume x PlatformHeadroom (device +0x320 =
//!   10^(PlatformHeadroomDB x 0.05), FAudioDevice::Init 0x2ef7154) x device master volume (1 here), clamped to [0, 4];
//! - the channel map (lib.rs `channel_map`, FMixerSource::UpdateChannelMaps 0x2cdb270).
//!
//! Mixer, per block of `BLOCK` output frames:
//! 1. FMixerSourceManager::ComputeSourceBuffersForIdRange 0x2cb7e60: pitch param target clamped to [0.0625, 16]; on
//!    the first block current = target, else it ramps linearly over the block (delta = (target - current) /
//!    NumFrames, current += delta before alpha += current); per output frame: while alpha >= 1 { frame index += 1;
//!    alpha -= 1 } then ReadSourceFrame 0x2ccce10 (also on the very first frame), out = cur + alpha * (next - cur)
//!    per channel; after the block current = target. ReadSourceFrame at the last frame of a buffer takes `next` from
//!    the next queued buffer (a looping wave's decoder continues at frame 0); with none queued it leaves `next` as it
//!    was (= the last frame) and sets the done flag (0x2ccd3b9), after which the block loop stops (0x2cb84ab).
//! 2. ComputePostSourceEffectBufferForIdRange 0x2cb7570 -> Audio::FadeBufferFast 0x2c80540 over the interleaved
//!    block (start = last volume, end = new volume): |end - start| <= 1e-8 -> x start (0 -> zeroed), else per group of
//!    4 samples gain = start + g * (end - start) / (NumSamples / 4); then last volume = end. The first volume has no
//!    fade: the SetVolume command (0x2cb43dc) sets the start too while the destination is still the -1 that
//!    ReleaseSource (0x2cce6b0) left.
//! 3. FMixerSourceSubmixOutputBuffer::ComputeOutput3D 0x2cb6d80 -> Audio::MixMonoTo2ChannelsFast 0x2c8b1b0 (mono
//!    sources; stereo: the 2x2 map the same way): per output frame i of the block, gain = start + (i + 1) x (end -
//!    start) / NumFrames per output channel, start = the previous map; SetChannelMap 0x2cd17b0 sets start = end the
//!    first time (+0x224).
//!
//! UNCONFIRMED: stereo 2x2 ramp (MixStereoTo2ChannelsFast not read; same form assumed), int16 -> float as x / 32768,
//! no submix chain (master submix only). Vorbis decode: lewton + the libvorbisfile rounding below.

use std::io::Cursor;
use std::sync::Arc;

/// DefaultEngine.ini WindowsTargetSettings AudioSampleRate
pub const MIXER_RATE: u32 = 48000;
/// DefaultEngine.ini WindowsTargetSettings AudioCallbackBufferFrameSize
pub const BLOCK: usize = 1024;
/// WindowsEngine.ini [Audio] PlatformHeadroomDB
pub const HEADROOM_DB: f32 = -3.0;
/// UAudioSettings ctor 0x2f07050 (+0x104 / +0x108)
pub const MIN_PITCH: f32 = 0.4;
pub const MAX_PITCH: f32 = 2.0;
/// FMixerSourceManager::ComputeSourceBuffersForIdRange 0x2cb8410 / 0x2cb8420
pub const SRC_MIN_PITCH: f32 = 0.0625;
pub const SRC_MAX_PITCH: f32 = 16.0;
/// FMixerSource::UpdateVolume 0x2cdcbf1
pub const MAX_VOLUME: f32 = 4.0;

/// FAudioDevice::Init 0x2ef7154: PlatformHeadroom = 10^(PlatformHeadroomDB x 0.05)
pub fn headroom() -> f32 {
    10f32.powf(HEADROOM_DB * 0.05)
}

/// decoded PCM, interleaved int16 (what UE's Vorbis decoder hands the mixer)
#[derive(Clone, Debug)]
pub struct Pcm {
    pub rate: u32,
    pub channels: u16,
    pub samples: Arc<Vec<i16>>,
}

impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / self.channels.max(1) as usize
    }
    pub fn seconds(&self) -> f64 {
        self.frames() as f64 / self.rate.max(1) as f64
    }
    /// sample (channel c of frame i) as float
    pub fn at(&self, i: usize, c: usize) -> f32 {
        self.samples[i * self.channels as usize + c] as f32 / 32768.0
    }
}

/// libvorbisfile ov_read (the shipped Engine/Binaries/ThirdParty/Vorbis/Win64/VS2015/libvorbisfile_64.dll, export
/// rva 0x51a0, 16-bit branch 0x54d0): vorbis_ftoi = cvtsd2si((double)(x * 32768.f)) (round to nearest, ties to even,
/// the default MXCSR), clipped to [-32768, 32767]
pub fn ov_read_sample(x: f32) -> i16 {
    let v = ((x * 32768.0) as f64).round_ties_even();
    v.clamp(-32768.0, 32767.0) as i16
}

/// the last Ogg page of a stream: (end-of-stream flag, granule position)
pub fn ogg_tail(bytes: &[u8]) -> Option<(bool, u64)> {
    let i = bytes.windows(4).rposition(|w| w == b"OggS")?;
    let h = bytes.get(i..i + 14)?;
    Some((h[5] & 4 != 0, u64::from_le_bytes(h[6..14].try_into().ok()?)))
}

/// the Ogg pages of a stream: (header type flags, granule position), in order
pub fn ogg_pages(bytes: &[u8]) -> Vec<(u8, u64)> {
    let mut out = vec![];
    let mut i = 0;
    while i + 27 <= bytes.len() && &bytes[i..i + 4] == b"OggS" {
        let n = bytes[i + 26] as usize;
        let Some(seg) = bytes.get(i + 27..i + 27 + n) else { break };
        let body: usize = seg.iter().map(|x| *x as usize).sum();
        out.push((bytes[i + 5], u64::from_le_bytes(bytes[i + 6..i + 14].try_into().unwrap())));
        i += 27 + n + body;
    }
    out
}

/// decode an Ogg Vorbis stream (a SoundWave's OGG bulk data) to interleaved int16 as ov_read hands it to UE's
/// FVorbisAudioInfo: lewton's float synthesis (a pure-Rust decoder of the same spec), ov_read_sample, and libvorbis'
/// granule trimming (vorbis_synthesis_blockin, libvorbis 1.3: on the first page with a granule position, samples
/// decoded beyond it are cut from the end when that page is also the end-of-stream page, else from the beginning;
/// at the end-of-stream page the output is cut to its granule position)
pub fn decode_ogg(bytes: &[u8]) -> Result<Pcm, String> {
    let mut r = lewton::inside_ogg::OggStreamReader::new(Cursor::new(bytes.to_vec())).map_err(|e| format!("ogg: {e:?}"))?;
    let channels = r.ident_hdr.audio_channels as u16;
    let ch = channels.max(1) as usize;
    let rate = r.ident_hdr.audio_sample_rate;
    // audio pages: those with a granule position (the header pages carry 0, -1 = no packet ends on the page)
    let pages: Vec<(u8, u64)> = ogg_pages(bytes).into_iter().filter(|p| p.1 != u64::MAX).collect();
    let last = pages.last().copied();
    let first_audio = pages.iter().find(|p| p.1 > 0).copied();
    let mut out = Vec::new();
    let mut begin_trimmed = false;
    while let Some(p) = r.read_dec_packet_generic::<lewton::samples::InterleavedSamples<f32>>().map_err(|e| format!("vorbis: {e:?}"))? {
        out.extend(p.samples.iter().map(|x| ov_read_sample(*x)));
        if !begin_trimmed {
            if let (Some(g), Some((flags, fg))) = (r.get_last_absgp(), first_audio) {
                begin_trimmed = true;
                let count = (out.len() / ch) as u64;
                if g == fg && count > g && flags & 4 == 0 {
                    out.drain(..((count - g) as usize * ch));
                }
            }
        }
    }
    if let Some((flags, g)) = last {
        if flags & 4 != 0 && out.len() > g as usize * ch {
            out.truncate(g as usize * ch);
        }
    }
    Ok(Pcm { rate, channels, samples: Arc::new(out) })
}

/// the game-side inputs of one playing wave (lib.rs writes them every game frame; the voice reads them per block)
#[derive(Clone, Copy, Debug)]
pub struct VoiceParams {
    /// cue volume x node volume
    pub volume: f32,
    /// distance attenuation gain
    pub gain: f32,
    /// cue pitch x node pitch
    pub pitch: f32,
    pub doppler: f32,
    /// the Enveloper's (volume, pitch) multipliers at the sound's playback time (game side)
    pub env: (f32, f32),
    /// channel map: gain of source channel c into output L / R
    pub map: [[f32; 2]; 2],
    /// the source's low-pass cutoff (Hz): the lowest of the class / attenuation / ambient zone cutoffs
    pub lpf: f32,
    /// the send level into the master reverb submix (reverb::send_level)
    pub reverb_send: f32,
    pub stop: bool,
}

impl Default for VoiceParams {
    fn default() -> Self {
        VoiceParams { volume: 1.0, gain: 1.0, pitch: 1.0, doppler: 1.0, env: (1.0, 1.0), map: [[0.5, 0.5], [0.0, 1.0]], lpf: crate::zones::MAX_LPF, reverb_send: 0.0, stop: false }
    }
}

pub struct Voice {
    pub pcm: Pcm,
    pub params: VoiceParams,
    /// loops left (-1 = forever, 0 = play once)
    loops: i64,
    // FSourceInfo: CurrentFrameIndex (+0xbc), CurrentFrameAlpha (+0xb8), Current/NextFrameValues (+0x98 / +0xa8)
    index: usize,
    alpha: f32,
    cur: Vec<f32>,
    next: Vec<f32>,
    read_first: bool,
    // PitchSourceParam (+0x110 current, +0x118 target, +0x120 init flag); VolumeSourceStart/Destination
    pitch_cur: f32,
    pitch_init: bool,
    vol_last: Option<f32>,
    /// the channel map the last block ended on (SetChannelMap start)
    map_last: Option<[[f32; 2]; 2]>,
    /// output frames rendered (playback time = rendered / MIXER_RATE)
    pub rendered: u64,
    pub finished: bool,
    block: Vec<f32>,
    lpf: InterpolatedLpf,
}

/// Audio::FInterpolatedLPF: a one-pole low-pass whose coefficient ramps across a block.
/// - Init 0x2c87a90: CutoffFrequency (+0) = -1, B1Curr (+4) = B1Delta (+8) = B1Target (+0xc) = 0, z1 per channel 0,
///   bIsFirstFrequencyChange (+0x38) = true;
/// - StartFrequencyInterpolation 0x2c9b180: the first call does not interpolate; on a cutoff change (> 1e-8)
///   B1Target = exp(-PI x clamp(2 f / rate, 0, 1)) and B1Delta = (B1Target - B1Curr) / frames; frames <= 1 jumps;
/// - ProcessAudioBuffer 0x2c92f50: per frame B1Curr += B1Delta, y = x + B1Curr (z1 - x), z1 = y (denormals to 0).
/// FMixerSourceManager::ComputePostSourceEffectBufferForIdRange 0x2cb7570 starts the interpolation every block with
/// the block's frame count (0x2cb7b4c) and bypasses the filter while the cutoff is >= 20000 (0x2cb7bdd).
#[derive(Clone, Debug)]
struct InterpolatedLpf {
    cutoff: f32,
    b1: f32,
    delta: f32,
    target: f32,
    first: bool,
    z1: Vec<f32>,
}

impl InterpolatedLpf {
    fn new(ch: usize) -> Self {
        InterpolatedLpf { cutoff: -1.0, b1: 0.0, delta: 0.0, target: 0.0, first: true, z1: vec![0.0; ch] }
    }

    fn start(&mut self, freq: f32, frames: usize) {
        let mut n = frames as i32;
        if self.first {
            n = 0;
            self.first = false;
        }
        if (freq - self.cutoff).abs() > 1e-8 {
            self.cutoff = freq;
            let norm = 2.0 * freq / MIXER_RATE as f32;
            let norm = if norm >= 0.0 { norm.min(1.0) } else { 0.0 };
            self.target = (-std::f32::consts::PI * norm).exp();
            self.delta = (self.target - self.b1) / n as f32;
        }
        if n <= 1 {
            self.b1 = self.target;
            self.delta = 0.0;
        }
    }

    fn process(&mut self, buf: &mut [f32]) {
        let ch = self.z1.len().max(1);
        for (i, x) in buf.iter_mut().enumerate() {
            let c = i % ch;
            if c == 0 {
                self.b1 += self.delta;
            }
            let mut y = *x + self.b1 * (self.z1[c] - *x);
            if y > -f32::MIN_POSITIVE && y < f32::MIN_POSITIVE {
                y = 0.0;
            }
            self.z1[c] = y;
            *x = y;
        }
    }
}

impl Voice {
    /// `loop_count`: None = once; Some(-1) = forever; Some(n) = n extra loops
    pub fn new(pcm: Pcm, params: VoiceParams, loop_count: Option<i64>) -> Voice {
        let ch = pcm.channels.max(1) as usize;
        Voice {
            pcm,
            params,
            loops: loop_count.unwrap_or(0),
            index: 0,
            alpha: 0.0,
            cur: vec![0.0; ch],
            next: vec![0.0; ch],
            read_first: false,
            pitch_cur: 1.0,
            pitch_init: false,
            vol_last: None,
            map_last: None,
            rendered: 0,
            finished: false,
            block: vec![],
            lpf: InterpolatedLpf::new(ch),
        }
    }

    /// the pitch and volume the source manager gets (FMixerSource::Update / UpdateVolume)
    pub fn targets(&self) -> (f32, f32) {
        let p = self.params.pitch * self.params.env.1 * self.params.doppler;
        // ClampPitch 0x2eeef70: x < min -> min, else min(max, x)
        let p = if p < MIN_PITCH { MIN_PITCH } else { p.min(MAX_PITCH) };
        let p = p * (self.pcm.rate as f32 / MIXER_RATE as f32);
        let v = self.params.volume * self.params.env.0 * self.params.gain * headroom();
        let v = if v < 0.0 { 0.0 } else { v.min(MAX_VOLUME) };
        (p, v)
    }

    /// ReadSourceFrame 0x2ccce10; false = no more frames (done)
    fn read_frame(&mut self) -> bool {
        let n = self.pcm.frames();
        let ch = self.pcm.channels as usize;
        while self.index >= n {
            if self.loops == 0 || n == 0 {
                return false;
            }
            if self.loops > 0 {
                self.loops -= 1;
            }
            self.index -= n;
        }
        for c in 0..ch {
            self.cur[c] = self.pcm.at(self.index, c);
            if self.index + 1 < n {
                self.next[c] = self.pcm.at(self.index + 1, c);
            } else if self.loops != 0 {
                self.next[c] = self.pcm.at(0, c);
            }
            // else: no buffer queued, `next` keeps its value (the last frame, read as `next` one frame earlier)
        }
        true
    }

    /// render one block of `frames` (<= BLOCK) output frames into the source's own channel layout (interleaved)
    pub fn render_source_block(&mut self, frames: usize) -> &[f32] {
        let ch = self.pcm.channels.max(1) as usize;
        self.block.clear();
        if self.finished || self.params.stop {
            self.finished = true;
            return &self.block;
        }
        let (pitch_target, vol) = self.targets();
        let target = if pitch_target < SRC_MIN_PITCH { SRC_MIN_PITCH } else { pitch_target.min(SRC_MAX_PITCH) };
        let delta = if self.pitch_init && frames > 0 {
            (target - self.pitch_cur) / frames as f32
        } else {
            self.pitch_cur = target;
            self.pitch_init = true;
            0.0
        };
        for _ in 0..frames {
            let first = !self.read_first;
            self.read_first = true;
            if self.alpha >= 1.0 {
                while self.alpha >= 1.0 {
                    self.index += 1;
                    self.alpha -= 1.0;
                }
                if !self.read_frame() {
                    self.finished = true;
                    break;
                }
            } else if first && !self.read_frame() {
                self.finished = true;
                break;
            }
            for c in 0..ch {
                self.block.push(self.cur[c] + self.alpha * (self.next[c] - self.cur[c]));
            }
            self.pitch_cur += delta;
            self.alpha += self.pitch_cur;
        }
        self.pitch_cur = target;
        let got = self.block.len() / ch;
        self.lpf.start(self.params.lpf, got);
        if self.lpf.cutoff < crate::zones::MAX_LPF {
            let mut b = std::mem::take(&mut self.block);
            self.lpf.process(&mut b);
            self.block = b;
        }
        let start = self.vol_last.unwrap_or(vol);
        fade_buffer_fast(&mut self.block, start, vol);
        self.vol_last = Some(vol);
        self.rendered += (self.block.len() / ch) as u64;
        &self.block
    }

    /// render up to `out.len() / 2` stereo frames at MIXER_RATE, mixed (added) into `out` (interleaved L R), block by
    /// block, through the channel map ramp (MixMonoTo2ChannelsFast)
    pub fn mix_stereo(&mut self, out: &mut [f32]) -> usize {
        self.mix_stereo_send(out, None)
    }

    /// mix_stereo, also adding the channel-mapped output x the reverb send level into `send` (the master reverb
    /// submix's input; FMixerSourceVoice::SetSubmixSendInfo 0x2cdb8f9 / 0x2cdb93e)
    pub fn mix_stereo_send(&mut self, out: &mut [f32], mut send: Option<&mut [f32]>) -> usize {
        let frames = out.len() / 2;
        let mut done = 0;
        while done < frames && !self.finished {
            let n = (frames - done).min(BLOCK);
            let ch = self.pcm.channels.max(1) as usize;
            let end = self.params.map;
            let start = self.map_last.unwrap_or(end);
            self.map_last = Some(end);
            let b = self.render_source_block(n).to_vec();
            let got = b.len() / ch;
            // the ramp spans the block's frame count
            for i in 0..got {
                let k = (i + 1) as f32 / n as f32;
                let mut lr = [0f32; 2];
                for c in 0..ch.min(2) {
                    for o in 0..2 {
                        let g = start[c][o] + k * (end[c][o] - start[c][o]);
                        lr[o] += b[i * ch + c] * g;
                    }
                }
                out[(done + i) * 2] += lr[0];
                out[(done + i) * 2 + 1] += lr[1];
                if let Some(sd) = send.as_deref_mut() {
                    let g = self.params.reverb_send;
                    if g > 0.0 {
                        sd[(done + i) * 2] += lr[0] * g;
                        sd[(done + i) * 2 + 1] += lr[1] * g;
                    }
                }
            }
            done += got;
            if got < n {
                break;
            }
        }
        done
    }
}

/// Audio::FadeBufferFast 0x2c80540 (buffer, NumSamples, Start, End)
pub fn fade_buffer_fast(buf: &mut [f32], start: f32, end: f32) {
    if (end - start).abs() <= 1e-8 {
        if start == 0.0 {
            buf.iter_mut().for_each(|x| *x = 0.0);
        } else {
            buf.iter_mut().for_each(|x| *x *= start);
        }
        return;
    }
    let groups = (buf.len() as i32) / 4;
    let d = (end - start) / groups as f32;
    let mut g = start;
    for chunk in buf.chunks_mut(4) {
        chunk.iter_mut().for_each(|x| *x *= g);
        g += d;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp_pcm(n: usize, rate: u32) -> Pcm {
        Pcm { rate, channels: 1, samples: Arc::new((0..n).map(|i| ((i % 2000) as i32 * 16 - 16000) as i16).collect()) }
    }

    /// pitch 1 at 48 kHz: the output is the decoded samples x volume exactly (alpha stays 0)
    #[test]
    fn unity_pitch_is_the_samples_times_volume() {
        let pcm = ramp_pcm(5000, 48000);
        let mut v = Voice::new(pcm.clone(), VoiceParams { volume: 0.5, gain: 0.8, ..Default::default() }, None);
        let mut all = vec![];
        while !v.finished {
            let b = v.render_source_block(BLOCK).to_vec();
            if b.is_empty() {
                break;
            }
            all.extend(b);
        }
        assert_eq!(all.len(), 5000);
        for (i, s) in all.iter().enumerate() {
            assert_eq!(*s, pcm.at(i, 0) * (0.5f32 * 0.8 * headroom()), "{i}");
        }
    }

    /// a 24 kHz wave advances half a source frame per output frame: odd outputs are the midpoints
    #[test]
    fn sample_rate_ratio_interpolates_linearly() {
        let pcm = ramp_pcm(3000, 24000);
        let mut v = Voice::new(pcm.clone(), VoiceParams { volume: 1.0 / headroom(), ..Default::default() }, None);
        let b = v.render_source_block(BLOCK).to_vec();
        let g = (1.0 / headroom()) * headroom();
        for i in 0..BLOCK / 2 - 1 {
            assert_eq!(b[2 * i], pcm.at(i, 0) * g);
            assert_eq!(b[2 * i + 1], (pcm.at(i, 0) + 0.5 * (pcm.at(i + 1, 0) - pcm.at(i, 0))) * g);
        }
    }

    /// pitch is clamped to [0.4, 2] (ClampPitch) and volume ramps across a block in groups of 4 (FadeBufferFast)
    #[test]
    fn clamp_and_fade() {
        let pcm = Pcm { rate: 48000, channels: 1, samples: Arc::new(vec![16384; 48000]) };
        let mut v = Voice::new(pcm, VoiceParams { pitch: 5.0, volume: 1.0 / headroom(), ..Default::default() }, None);
        assert_eq!(v.targets().0, 2.0);
        v.params.pitch = 1.0;
        v.render_source_block(BLOCK);
        v.params.volume = 0.0;
        let b = v.render_source_block(BLOCK).to_vec();
        assert!(b[0..4].iter().all(|x| (x - 0.5).abs() < 1e-6));
        let d = -1.0 / 256.0;
        assert!((b[4] - 0.5 * (1.0 + d)).abs() < 1e-6 && (b[1023] - 0.5 * (1.0 + 255.0 * d)).abs() < 1e-6);
        let b = v.render_source_block(BLOCK).to_vec();
        assert!(b.iter().all(|x| *x == 0.0));
    }

    /// a non-looping wave's last frame holds (ReadSourceFrame leaves `next`), then the voice stops
    #[test]
    fn end_holds_last_frame() {
        let pcm = Pcm { rate: 24000, channels: 1, samples: Arc::new(vec![0, 16384, 32767]) };
        let mut v = Voice::new(pcm.clone(), VoiceParams { volume: 1.0 / headroom(), ..Default::default() }, None);
        let b = v.render_source_block(BLOCK).to_vec();
        // frames 0, .5, 1, 1.5, 2, 2.5 (held), then done
        assert_eq!(b.len(), 6);
        assert!((b[4] - pcm.at(2, 0)).abs() < 1e-6 && (b[5] - pcm.at(2, 0)).abs() < 1e-6);
        assert!(v.finished);
    }

    /// the channel map ramps per output frame from the last map to the new one (MixMonoTo2ChannelsFast)
    #[test]
    fn channel_map_ramps_per_frame() {
        let pcm = Pcm { rate: 48000, channels: 1, samples: Arc::new(vec![16384; 4096]) };
        let mut v = Voice::new(pcm, VoiceParams { volume: 1.0 / headroom(), map: [[1.0, 0.0], [0.0, 1.0]], ..Default::default() }, None);
        let mut out = vec![0f32; BLOCK * 2];
        v.mix_stereo(&mut out);
        assert!((out[0] - 0.5).abs() < 1e-6 && out[1] == 0.0);
        v.params.map = [[0.0, 1.0], [0.0, 1.0]];
        let mut out = vec![0f32; BLOCK * 2];
        v.mix_stereo(&mut out);
        let k = 1.0 / BLOCK as f32;
        assert!((out[0] - 0.5 * (1.0 - k)).abs() < 1e-5 && (out[1] - 0.5 * k).abs() < 1e-5);
        assert!(out[(BLOCK - 1) * 2].abs() < 1e-5 && (out[(BLOCK - 1) * 2 + 1] - 0.5).abs() < 1e-5);
    }
}

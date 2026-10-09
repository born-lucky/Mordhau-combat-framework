//! UE's plate reverb (Audio::FPlateReverbFast) as the exe runs it on the master reverb submix, with the AudioVolume
//! reverb settings that drive it.
//!
//! Parameter path:
//! - the listener's audio volume (zones::zone_at) -> its FReverbSettings (bApplyReverb, ReverbEffect, Volume,
//!   FadeTime); on a volume change FAudioDevice::UpdateAudioVolumeEffects 0x2f01384 calls
//!   FAudioEffectsManager::SetReverbSettings 0x2f205e0 (source = the current effect, destination = the asset's
//!   values, Volume 0 when there is no effect or bApplyReverb is off, destination time = now + FadeTime);
//! - FAudioEffectsManager::Update 0x2f234d0 -> FAudioReverbEffect::Interpolate 0x2f1a3f0 -> SetReverbEffectParameters
//!   -> FSubmixEffectReverb::SetParameters 0x2cd1f30 (FAudioReverbEffect -> FPlateReverbFastSettings);
//! - FSubmixEffectReverb::OnProcessAudio 0x2cc9310: the input is ramped from the previous to the new Volume
//!   (MixInBufferFast), then FPlateReverbFast::ProcessAudio 0x2c90e20 (early + late reflections, summed), interleaved
//!   by InterleaveAndMixOutput 0x2c89720. The submix asks for 2 input channels (GetDesiredInputChannelCountOverride
//!   0x7d7430).
//!
//! DSP (UE 4.25 SignalProcessing, every constant read from the exe):
//! - early reflections: FEarlyReflectionsFast ctor 0x2c722f0, ApplySettings 0x2c7a460, ProcessAudio 0x2c8f2d0: per
//!   channel pre-delay (FIntegerDelay) -> one-pole LPF -> 4-line feedback delay network (FFeedbackDelayNetwork ctor
//!   0x2c725f0, ProcessAudioBuffer 0x2c92470) -> x Gain;
//! - late reflections: FLateReflectionsFast ctor 0x2c73ab0, ApplySettings 0x2c7a7e0, ProcessAudioBuffer 0x2c931c0:
//!   (L + R) x Bandwidth x Gain / 2 -> pre-delay -> LPF -> 4 all-passes (FLongDelayAPF block 0x2c91d60) -> two plates
//!   (FLateReflectionsPlate ctor 0x2c745c0, ProcessAudioFrames 0x2c94e10) cross-fed through their last delay
//!   (PeekDelayLine 0x2c8cd20), modulated by GeneraterPlateModulations 0x2c84cc0 (FDynamicDelayAPF 0x2c71e00 /
//!   0x2c8efb0 / 0x2c91620 over FLinearInterpFractionalDelay 0x2c74fd0 / 0x2c91a90), output taps summed / subtracted.
//!
//! The exe runs these in blocks (delay lines as FAlignedBlockBuffers read before written, every block no longer
//! than the shortest delay); a per-sample ring buffer gives the same samples. Kept block-wise: the dynamic APF's
//! all-pass gain ramp per block (FadeBufferFast 0x2c80540 steps once per 4 samples) and the wet volume ramp.

use serde_json::Value;

pub const SR: f32 = crate::mixer::MIXER_RATE as f32;
/// FSubmixEffectReverb::Init 0x2cc3135: FPlateReverbFast(SampleRate, 512, ...)
const MAX_BLOCK: usize = 512;

fn flush(x: f32) -> f32 {
    if x > -f32::MIN_POSITIVE && x < f32::MIN_POSITIVE { 0.0 } else { x }
}

/// FadeBufferFast 0x2c80540: |start - end| <= 1e-8 -> x start (0 -> zeros); else the gain steps by
/// (end - start) / (n / 4) once per 4 samples
pub fn fade_gains(n: usize, start: f32, end: f32) -> impl Fn(usize) -> f32 {
    let groups = (n / 4).max(1) as f32;
    let flat = (start - end).abs() <= 1e-8;
    let step = (end - start) / groups;
    move |i| if flat { start } else { start + step * (i / 4) as f32 }
}

/// an integer delay line: tick(x) returns the sample pushed `len` samples ago (len 0 = pass-through)
#[derive(Clone, Debug)]
struct Delay {
    buf: Vec<f32>,
    w: usize,
}

impl Delay {
    fn new(len: usize) -> Delay {
        Delay { buf: vec![0.0; len], w: 0 }
    }
    fn peek(&self) -> f32 {
        self.buf.get(self.w).copied().unwrap_or(0.0)
    }
    fn tick(&mut self, x: f32) -> f32 {
        if self.buf.is_empty() {
            return x;
        }
        let y = self.buf[self.w];
        self.buf[self.w] = x;
        self.w = (self.w + 1) % self.buf.len();
        y
    }
    /// FIntegerDelay::SetDelayLengthSamples (ER ApplySettings 0x2c7a466..): the line keeps its history, the read
    /// position moves (a longer delay reads older samples, zeros before the line's start)
    fn set_len(&mut self, len: usize, max: usize) {
        let len = len.min(max);
        if len == self.buf.len() {
            return;
        }
        // newest-first history
        let n = self.buf.len();
        let hist: Vec<f32> = (0..n).map(|k| self.buf[(self.w + n - 1 - k) % n.max(1)]).collect();
        let mut b = vec![0.0; len];
        for k in 0..len.min(n) {
            // slot holding the sample k+1 pushes ago, read when `len - k - 1` more pushes have happened
            b[len - 1 - k] = hist[k];
        }
        self.buf = b;
        self.w = 0;
    }
    fn clear(&mut self) {
        self.buf.iter_mut().for_each(|x| *x = 0.0);
    }
}

/// FBufferOnePoleLPF (ProcessAudio 0x2c8e670): y = A0 x + B1 z1 (+4 B1, +8 A0, +0xc Z1)
#[derive(Clone, Copy, Debug, Default)]
struct Lpf {
    b1: f32,
    a0: f32,
    z1: f32,
}

impl Lpf {
    fn tick(&mut self, x: f32) -> f32 {
        let y = self.a0 * x + self.b1 * self.z1;
        self.z1 = y;
        flush(y)
    }
}

/// a Schroeder all-pass around a delay line (FLongDelayAPF::ProcessAudioBlock 0x2c91d60, the FDN lines
/// 0x2c925e0..): w = x + G D (into the line), y = D - G w
#[derive(Clone, Debug)]
struct Apf {
    g: f32,
    line: Delay,
}

impl Apf {
    fn new(g: f32, len: usize) -> Apf {
        Apf { g, line: Delay::new(len) }
    }
    /// (output, the sample written into the line)
    fn tick(&mut self, x: f32) -> (f32, f32) {
        let d = self.line.peek();
        let w = flush(x + self.g * d);
        self.line.tick(w);
        (d - self.g * w, w)
    }
}

/// FFeedbackDelayNetwork with the ER coefficients (layout +0x14: input gain, 4 APF gains, 4 LPF A0, 4 LPF B1,
/// feedback gain; state +0x4c Z[4], +0x5c F[4])
#[derive(Clone, Debug)]
struct Fdn {
    lines: [Delay; 4],
    input: f32,
    g: [f32; 4],
    a0: [f32; 4],
    b1: [f32; 4],
    fb: f32,
    z: [f32; 4],
    f: [f32; 4],
}

impl Fdn {
    fn new(d: [usize; 4]) -> Fdn {
        Fdn { lines: d.map(Delay::new), input: 0.25, g: [0.1, 0.2, 0.3, 0.4], a0: [1.0; 4], b1: [0.0; 4], fb: 0.0, z: [0.0; 4], f: [0.0; 4] }
    }
    fn clear(&mut self) {
        self.lines.iter_mut().for_each(Delay::clear);
        self.z = [0.0; 4];
        self.f = [0.0; 4];
    }
    /// ProcessAudioBuffer 0x2c925e0..0x2c92800
    fn tick(&mut self, x: f32) -> f32 {
        let x = x * self.input;
        let mut out = 0.0;
        for k in 0..4 {
            let d = self.lines[k].peek();
            let a = flush(d * self.g[k] + (x + self.f[k]));
            self.lines[k].tick(a);
            let y = d - a * self.g[k];
            let z = flush(y * self.a0[k] + self.z[k] * self.b1[k]);
            self.z[k] = z;
            out += z;
        }
        let [z0, z1, z2, z3] = self.z;
        // 0x2c927a7..0x2c927fa
        self.f = [(z2 - z1) * self.fb, (z0 + z3) * self.fb, (z0 - z3) * self.fb, (-z1 - z2) * self.fb];
        out
    }
}

/// FEarlyReflectionsFastSettings (ClampSettings 0x2c7d7f0)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EarlySettings {
    pub gain: f32,
    pub pre_delay_msec: f32,
    pub bandwidth: f32,
    pub decay: f32,
    pub absorption: f32,
}

impl EarlySettings {
    fn clamped(mut self) -> Self {
        self.gain = if self.gain >= 0.0 { self.gain.min(0.9999) } else { 0.0 };
        self.pre_delay_msec = if self.pre_delay_msec >= 0.0 { self.pre_delay_msec.min(1000.0) } else { 0.0 };
        self.bandwidth = if self.bandwidth >= 0.0 { self.bandwidth.min(0.99999) } else { 0.0 };
        self.decay = if self.decay >= 0.0001 { self.decay.min(1.0) } else { 0.0001 };
        if self.absorption < 0.0 {
            self.absorption = 0.0;
        }
        self
    }
}

#[derive(Clone, Debug)]
struct Early {
    pre: [Delay; 2],
    lpf: [Lpf; 2],
    fdn: [Fdn; 2],
    gain: f32,
}

impl Early {
    /// ctor 0x2c722f0: FDN delays SR x {0.0238567, 0.0179765, 0.0636739, 0.0465374} (left) and {0.0468734,
    /// 0.0230167, 0.0357851, 0.0549377} (right); pre-delays of up to 2 x int(SR)
    fn new(s: EarlySettings) -> Early {
        let d = |c: f32| (SR * c) as usize;
        let mut e = Early {
            pre: [Delay::new(0), Delay::new(0)],
            lpf: [Lpf::default(); 2],
            fdn: [Fdn::new([d(0.0238567), d(0.0179765), d(0.0636739), d(0.0465374)]), Fdn::new([d(0.0468734), d(0.0230167), d(0.0357851), d(0.0549377)])],
            gain: 1.0,
        };
        e.apply(s.clamped());
        e
    }

    fn max_pre() -> usize {
        ((SR as i32) as f32 * 2.0) as usize
    }

    /// ApplySettings 0x2c7a460
    fn apply(&mut self, s: EarlySettings) {
        let pre = ((SR as i32) as f32 * s.pre_delay_msec * 0.001) as i32;
        for p in &mut self.pre {
            p.set_len(pre.max(0) as usize, Self::max_pre());
        }
        // 0x2c7a576: B1 = log2(1 + Bandwidth), A0 = 1 - B1
        let b1 = (s.bandwidth + 1.0).ln() * std::f32::consts::LOG2_E;
        for l in &mut self.lpf {
            l.b1 = b1;
            l.a0 = 1.0 - b1;
        }
        let a = s.absorption;
        let fb = (1.0 - s.decay) * 0.5;
        // left: absorption + {0.1, -0.12, 0.08, -0.07}, each min 0.9999; right: + {0.17, -0.07, 0.05, -0.11}, min 0.999
        let lb = [(a + 0.1).min(0.9999), (a - 0.12).min(0.9999), (a + 0.08).min(0.9999), (a - 0.07).min(0.9999)];
        let rb = [(a + 0.17).min(0.999), (a - 0.07).min(0.999), (a + 0.05).min(0.999), (a - 0.11).min(0.999)];
        for (f, b) in self.fdn.iter_mut().zip([lb, rb]) {
            f.input = 0.25;
            f.g = [0.1, 0.2, 0.3, 0.4];
            f.b1 = b;
            f.a0 = b.map(|x| 1.0 - x);
            f.fb = fb;
        }
        self.gain = s.gain;
    }

    fn clear(&mut self) {
        self.pre.iter_mut().for_each(Delay::clear);
        self.fdn.iter_mut().for_each(Fdn::clear);
    }

    /// (left, right) of one stereo input frame
    fn tick(&mut self, l: f32, r: f32) -> (f32, f32) {
        let mut o = [0.0; 2];
        for (c, x) in [l, r].into_iter().enumerate() {
            let p = self.pre[c].tick(x);
            let f = self.lpf[c].tick(p);
            o[c] = self.fdn[c].tick(f) * self.gain;
        }
        (o[0], o[1])
    }
}

/// FLateReflectionsFastSettings (ClampSettings 0x2c7d890)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LateSettings {
    pub late_delay_msec: f32,
    pub late_gain_db: f32,
    pub bandwidth: f32,
    pub diffusion: f32,
    pub dampening: f32,
    pub decay: f32,
    pub density: f32,
}

impl LateSettings {
    fn clamped(mut self) -> Self {
        let c = |x: f32| if x >= 0.0 { x.min(0.99999) } else { 0.0 };
        self.late_delay_msec = if self.late_delay_msec >= 0.0 { self.late_delay_msec.min(2000.0) } else { 0.0 };
        self.late_gain_db = self.late_gain_db.min(0.0);
        self.bandwidth = c(self.bandwidth);
        self.dampening = c(self.dampening);
        self.diffusion = c(self.diffusion);
        self.decay = if self.decay >= 0.0001 { self.decay.min(1.0) } else { 0.0001 };
        if self.density < 0.0 {
            self.density = 0.0;
        }
        self
    }
}

/// FDynamicDelayAPF: an all-pass whose line is read through a linearly interpolated fractional delay of
/// clamp(mod, MinDelay - 1, MaxDelay) samples; its gain eases linearly over DurationSeconds (1 s) x SampleRate
/// samples (FLinearEase at +0..+0x20; SetDensity 0x2c98cd0)
#[derive(Clone, Debug)]
struct DynApf {
    hist: Vec<f32>,
    w: usize,
    min: usize,
    max: usize,
    g: f32,
    ease_start: f32,
    ease_delta: f32,
    ease_steps: usize,
    ease_step: usize,
}

impl DynApf {
    fn new(min: usize, max: usize) -> DynApf {
        DynApf { hist: vec![0.0; max + 4], w: 0, min, max, g: 0.0, ease_start: 0.0, ease_delta: 0.0, ease_steps: 0, ease_step: 0 }
    }
    /// SetDensity 0x2c98cf7: not the first change (the ctor clears bIsInit, 0x2c71e99) -> ease from the current gain
    fn set_g(&mut self, target: f32) {
        self.ease_steps = (1.0 * SR) as usize;
        self.ease_step = 0;
        if self.ease_steps == 0 {
            self.g = target;
        } else {
            self.ease_start = self.g;
            self.ease_delta = target - self.g;
        }
    }
    /// the gain at the start and the end of an n-sample block (ProcessAudioBlock 0x2c917d1..0x2c9183b)
    fn block_gains(&mut self, n: usize) -> (f32, f32) {
        if self.ease_step >= self.ease_steps {
            return (self.g, self.g);
        }
        let steps = self.ease_steps as f32;
        let g0 = self.ease_start + self.ease_delta * self.ease_step as f32 / steps;
        self.ease_step += 1;
        self.g = g0;
        if self.ease_step >= self.ease_steps {
            return (g0, g0);
        }
        self.ease_step = (self.ease_step - 1 + n).min(self.ease_steps);
        let g1 = self.ease_start + self.ease_delta * self.ease_step as f32 / steps;
        self.g = g1;
        (g0, g1)
    }
    fn clear(&mut self) {
        self.hist.iter_mut().for_each(|x| *x = 0.0);
    }
    /// the line read `d` samples back (d >= 1: the newest written sample is 1 back)
    fn back(&self, d: usize) -> f32 {
        let n = self.hist.len();
        if d > n {
            return 0.0;
        }
        self.hist[(self.w + n - d) % n]
    }
    fn tick(&mut self, x: f32, m: f32, g: f32, ng: f32) -> f32 {
        // FDynamicDelayAPF::ProcessAudio 0x2c8f0c0: mod - (MinDelay - 1); FLinearInterpFractionalDelay 0x2c91bf7:
        // clamp to [0, MaxDelay - MinDelay + 1], y = (1 - frac) x[d0] + frac x[d0 + 1]
        let base = self.min.saturating_sub(1);
        let span = (self.max - self.min + 1) as f32;
        let fd = (m - base as f32).clamp(0.0, span);
        let fl = fd.floor();
        let fr = fd - fl;
        let d0 = base + fl as usize;
        let d = (1.0 - fr) * self.back(d0.max(1)) + fr * self.back(d0 + 1);
        let w = flush(x + d * g);
        let n = self.hist.len();
        self.hist[self.w] = w;
        self.w = (self.w + 1) % n;
        d + w * ng
    }
}

/// FLateReflectionsPlate
#[derive(Clone, Debug)]
struct Plate {
    dyn_apf: DynApf,
    d: [Delay; 4],
    lpf: Lpf,
    apf: Apf,
    t: [Delay; 5],
    dampening: f32,
    decay: f32,
}

impl Plate {
    /// ctor 0x2c745c0 with FLateReflectionsPlateDelays `k` (x SR): [0] modulated base, [1] modulation depth,
    /// [2..5] the delays after the dynamic APF, [6] the long APF, [7..11] the output taps
    fn new(k: [usize; 12]) -> Plate {
        Plate {
            dyn_apf: DynApf::new(k[0] - k[1], k[0] + k[1]),
            d: [Delay::new(k[2]), Delay::new(k[3]), Delay::new(k[4]), Delay::new(k[5])],
            lpf: Lpf { b1: 0.0005, a0: 1.0, z1: 0.0 },
            apf: Apf::new(0.0, k[6]),
            t: [Delay::new(k[7]), Delay::new(k[8]), Delay::new(k[9]), Delay::new(k[10]), Delay::new(k[11])],
            dampening: 0.0005,
            decay: 0.5,
        }
    }
    fn clear(&mut self) {
        self.dyn_apf.clear();
        self.d.iter_mut().for_each(Delay::clear);
        self.apf.line.clear();
        self.t.iter_mut().for_each(Delay::clear);
        self.lpf.z1 = 0.0;
    }
    /// the sample the last tap delay outputs this tick (PeekDelayLine 0x2c8cd20)
    fn peek(&self) -> f32 {
        self.t[4].peek()
    }
    /// ProcessAudioFrames 0x2c94e10; returns the 8 output taps
    fn tick(&mut self, x: f32, fb: f32, m: f32, g: f32, ng: f32) -> [f32; 8] {
        let a = x + fb * (1.0 - self.decay);
        let b = self.dyn_apf.tick(a, m, g, ng);
        let o0 = self.d[0].tick(b);
        let o1 = self.d[1].tick(o0);
        let o2 = self.d[2].tick(o1);
        let a = self.d[3].tick(o2) * (1.0 - self.dampening);
        let b = self.lpf.tick(a) * (1.0 - self.decay);
        let (y, w) = self.apf.tick(b);
        let o3 = self.t[0].tick(w);
        let o4 = self.t[1].tick(o3);
        let o5 = self.t[2].tick(y);
        let o6 = self.t[3].tick(o5);
        let o7 = self.t[4].tick(o6);
        [o0, o1, o2, o3, o4, o5, o6, o7]
    }
    /// SetDensity 0x2c98cd0 + LR ApplySettings 0x2c7a8ef..
    fn apply(&mut self, s: &LateSettings) {
        let g = (-s.density).clamp(-0.9, 0.9);
        self.dyn_apf.set_g(g);
        self.apf.g = s.density - 0.15;
        self.dampening = s.dampening;
        self.lpf.b1 = s.dampening;
        self.lpf.a0 = 1.0 - s.dampening;
        self.decay = s.decay;
    }
}

#[derive(Clone, Debug)]
struct Late {
    gain: f32,
    bandwidth: f32,
    pre: Delay,
    lpf: Lpf,
    apf: [Apf; 4],
    plates: [Plate; 2],
    /// modulation base / depth per plate, LFO phases (+8 / +0xc) and increment (+0x10)
    mod_base: [f32; 2],
    mod_depth: [f32; 2],
    phase: [f32; 2],
    inc: f32,
}

impl Late {
    /// ctor 0x2c73ab0 (plate delays 0x2c73b2c..0x2c73cdb, input APFs 0x2c74041..0x2c743a0)
    fn new(s: LateSettings) -> Late {
        let d = |c: f32| (SR * c) as usize;
        let depth = d(0.000537616);
        let left = [d(0.0305097), depth, d(0.0118612), d(0.0281241), d(0.0818857), d(0.0198246), d(0.0892443), d(0.00628339), d(0.0349787), d(0.0358187), d(0.0539968), d(0.0155573)];
        let right = [d(0.0225799), depth, d(0.00893787), d(0.0619939), d(0.0289977), d(0.0496959), d(0.0604818), d(0.0112563), d(0.0530224), d(0.00406572), d(0.0630019), d(0.0579282)];
        let mut l = Late {
            gain: 1.0,
            bandwidth: 0.0,
            pre: Delay::new(0),
            lpf: Lpf::default(),
            apf: [Apf::new(s.diffusion, d(0.00477134)), Apf::new(s.diffusion, d(0.00359531)), Apf::new(s.diffusion - 0.125, d(0.0127348)), Apf::new(s.diffusion - 0.125, d(0.00930748))],
            plates: [Plate::new(left), Plate::new(right)],
            mod_base: [left[0] as f32, right[0] as f32],
            mod_depth: [depth as f32; 2],
            phase: [0.0, std::f32::consts::FRAC_PI_2],
            inc: std::f32::consts::TAU / SR,
        };
        l.apply(s.clamped());
        l
    }

    /// ApplySettings 0x2c7a7e0
    fn apply(&mut self, s: LateSettings) {
        self.gain = 10f32.powf(s.late_gain_db * 0.05);
        let max = 8 + (SR * 2.0) as usize;
        self.pre.set_len((s.late_delay_msec * SR * 0.001).max(0.0) as usize, max);
        self.bandwidth = s.bandwidth;
        self.lpf.b1 = 1.0 - s.bandwidth;
        self.lpf.a0 = s.bandwidth;
        self.apf[0].g = s.diffusion;
        self.apf[1].g = s.diffusion;
        self.apf[2].g = s.diffusion - 0.125;
        self.apf[3].g = s.diffusion - 0.125;
        for p in &mut self.plates {
            p.apply(&s);
        }
    }

    fn clear(&mut self) {
        self.pre.clear();
        self.lpf.z1 = 0.0;
        self.apf.iter_mut().for_each(|a| a.line.clear());
        self.plates.iter_mut().for_each(Plate::clear);
    }

    /// GeneraterPlateModulations 0x2c84de0: 0.5 + p (2/pi - 2/pi^2 |p|) at the current phase, then the phase advances
    /// and wraps past pi
    fn modulation(&mut self, k: usize) -> f32 {
        let p = self.phase[k];
        let s = (0.63662 - p.abs() * 0.202642) * p + 0.5;
        let mut q = p + self.inc;
        if q > std::f32::consts::PI {
            q -= std::f32::consts::TAU;
        }
        self.phase[k] = q;
        self.mod_base[k] + self.mod_depth[k] * s
    }

    /// one block of stereo frames (ProcessAudioBuffer 0x2c931c0), `n` <= MAX_BLOCK; adds into `out` (interleaved)
    fn process(&mut self, input: &[f32], out: &mut [f32]) {
        let n = input.len() / 2;
        let (g0, g1) = (self.plates[0].dyn_apf.block_gains(n), self.plates[1].dyn_apf.block_gains(n));
        let gl = fade_gains(n, g0.0, g0.1);
        let gr = fade_gains(n, g1.0, g1.1);
        let ngl = fade_gains(n, -g0.0, -g0.1);
        let ngr = fade_gains(n, -g1.0, -g1.1);
        // 0x2c93327: x Bandwidth x Gain / NumChannels
        let scale = self.bandwidth * self.gain / 2.0;
        for i in 0..n {
            let x = (input[2 * i] + input[2 * i + 1]) * scale;
            let mut x = self.lpf.tick(self.pre.tick(x));
            for a in &mut self.apf {
                x = a.tick(x).0;
            }
            let (ml, mr) = (self.modulation(0), self.modulation(1));
            let (fl, fr) = (self.plates[1].peek(), self.plates[0].peek());
            let l = self.plates[0].tick(x, fl, ml, gl(i), ngl(i));
            let r = self.plates[1].tick(x, fr, mr, gr(i), ngr(i));
            // 0x2c9343a..0x2c936c4
            out[2 * i] += r[0] + r[2] - r[4] + r[6] - l[1] - l[3] - l[5];
            out[2 * i + 1] += l[0] + l[2] - l[4] + l[6] - r[1] - r[3] - r[5];
        }
    }
}

/// FPlateReverbFastSettings (ctor 0x2c75ac0)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PlateSettings {
    pub early: EarlySettings,
    pub late: LateSettings,
    pub enable_early: bool,
    pub enable_late: bool,
}

impl Default for PlateSettings {
    fn default() -> Self {
        PlateSettings {
            early: EarlySettings { gain: 1.0, pre_delay_msec: 0.0, bandwidth: 0.8, decay: 0.5, absorption: 0.7 },
            late: LateSettings { late_delay_msec: 0.0, late_gain_db: 0.0, bandwidth: 0.5, diffusion: 0.5, dampening: 0.5, decay: 0.5, density: 0.5 },
            enable_early: true,
            enable_late: true,
        }
    }
}

/// FAudioReverbEffect (UReverbEffect's values + Volume); field order as the exe's struct (+0x10 Volume .. +0x45)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ReverbEffect {
    pub volume: f32,
    pub density: f32,
    pub diffusion: f32,
    pub gain: f32,
    pub gain_hf: f32,
    pub decay_time: f32,
    pub decay_hf_ratio: f32,
    pub reflections_gain: f32,
    pub reflections_delay: f32,
    pub late_gain: f32,
    pub late_delay: f32,
    pub air_absorption_gain_hf: f32,
    pub room_rolloff: f32,
    pub bypass_early: bool,
    pub bypass_late: bool,
}

impl Default for ReverbEffect {
    /// UReverbEffect ctor 0x33c28a0 (bBypassEarlyReflections +0x28 = true), Volume 0
    fn default() -> Self {
        ReverbEffect {
            volume: 0.0,
            density: 1.0,
            diffusion: 1.0,
            gain: 0.32,
            gain_hf: 0.89,
            decay_time: 1.49,
            decay_hf_ratio: 0.83,
            reflections_gain: 0.05,
            reflections_delay: 0.007,
            late_gain: 1.26,
            late_delay: 0.011,
            air_absorption_gain_hf: 0.994,
            room_rolloff: 0.0,
            bypass_early: true,
            bypass_late: false,
        }
    }
}

impl ReverbEffect {
    /// a ReverbEffect asset's properties over the ctor defaults
    pub fn from_asset(p: &Value) -> ReverbEffect {
        let d = ReverbEffect::default();
        let f = |k: &str, x: f32| p.get(k).and_then(Value::as_f64).map_or(x, |y| y as f32);
        let b = |k: &str, x: bool| p.get(k).and_then(Value::as_bool).unwrap_or(x);
        ReverbEffect {
            volume: 0.0,
            density: f("Density", d.density),
            diffusion: f("Diffusion", d.diffusion),
            gain: f("Gain", d.gain),
            gain_hf: f("GainHF", d.gain_hf),
            decay_time: f("DecayTime", d.decay_time),
            decay_hf_ratio: f("DecayHFRatio", d.decay_hf_ratio),
            reflections_gain: f("ReflectionsGain", d.reflections_gain),
            reflections_delay: f("ReflectionsDelay", d.reflections_delay),
            late_gain: f("LateGain", d.late_gain),
            late_delay: f("LateDelay", d.late_delay),
            air_absorption_gain_hf: f("AirAbsorptionGainHF", d.air_absorption_gain_hf),
            room_rolloff: f("RoomRolloffFactor", d.room_rolloff),
            bypass_early: b("bBypassEarlyReflections", d.bypass_early),
            bypass_late: b("bBypassLateReflections", d.bypass_late),
        }
    }

    /// FAudioReverbEffect::Interpolate 0x2f1a3f0 at fraction t in (0, 1): every value lerps, the early group
    /// (GainHF, ReflectionsGain, ReflectionsDelay) counting 0 on a side that bypasses early reflections, the late
    /// group (the rest but Volume) 0 on a side that bypasses late; a bypass holds only if both sides bypass
    pub fn lerp(src: &ReverbEffect, dst: &ReverbEffect, t: f32) -> ReverbEffect {
        let l = |a: f32, b: f32| (1.0 - t) * a + t * b;
        let e = |a: f32, b: f32| l(if src.bypass_early { 0.0 } else { a }, if dst.bypass_early { 0.0 } else { b });
        let la = |a: f32, b: f32| l(if src.bypass_late { 0.0 } else { a }, if dst.bypass_late { 0.0 } else { b });
        ReverbEffect {
            volume: l(src.volume, dst.volume),
            gain_hf: e(src.gain_hf, dst.gain_hf),
            reflections_gain: e(src.reflections_gain, dst.reflections_gain),
            reflections_delay: e(src.reflections_delay, dst.reflections_delay),
            density: la(src.density, dst.density),
            diffusion: la(src.diffusion, dst.diffusion),
            gain: la(src.gain, dst.gain),
            decay_time: la(src.decay_time, dst.decay_time),
            decay_hf_ratio: la(src.decay_hf_ratio, dst.decay_hf_ratio),
            late_gain: la(src.late_gain, dst.late_gain),
            late_delay: la(src.late_delay, dst.late_delay),
            air_absorption_gain_hf: la(src.air_absorption_gain_hf, dst.air_absorption_gain_hf),
            room_rolloff: la(src.room_rolloff, dst.room_rolloff),
            bypass_early: src.bypass_early && dst.bypass_early,
            bypass_late: src.bypass_late && dst.bypass_late,
        }
    }

    /// FSubmixEffectReverb::SetParameters 0x2cd1f30: the plate settings and the wet volume
    pub fn plate_settings(&self) -> (PlateSettings, f32) {
        let c = |x: f32| if x >= 0.0 { x.min(1.0) } else { 0.0 };
        let mut s = PlateSettings::default();
        s.early.gain = c(self.reflections_gain * 0.316456);
        s.early.pre_delay_msec = c(self.reflections_delay * 3.333333) * 300.0;
        s.early.bandwidth = c(1.0 - self.gain_hf);
        s.late.late_delay_msec = c(self.late_delay * 10.0) * 100.0;
        // LateGainDB = 20 log10(max(clamp01(Gain), 1e-8)) (0x2cd20ec: ln x 8.68589)
        s.late.late_gain_db = c(self.gain).max(1e-8).ln() * 8.685889;
        s.late.bandwidth = c(self.air_absorption_gain_hf) * 0.5 + 0.1;
        s.late.diffusion = c((self.diffusion - 0.05) * 1.052632) * 0.95;
        s.late.dampening = c((self.decay_hf_ratio - 0.05) * 0.526316) * 0.999;
        s.late.density = c(self.density * 1.052632) * 0.94 + 0.06;
        s.late.decay = decay_curve(self.decay_time);
        s.enable_early = !self.bypass_early;
        s.enable_late = !self.bypass_late;
        (s, self.volume)
    }
}

/// the decay curve FSubmixEffectReverb::Init builds (0x2cc2fed..0x2cc3119): keys (DecayTime, Decay) (0, 0.99),
/// (2, 0.45), (5, 0.15), (10, 0.1), (18, 0.01), (19, 0.002), (20, 0.0001); AddKey's keys are RCIM_Linear
/// (UNCONFIRMED: FRichCurveKey ctor default not read), constant outside
pub fn decay_curve(t: f32) -> f32 {
    const K: [(f32, f32); 7] = [(0.0, 0.99), (2.0, 0.45), (5.0, 0.15), (10.0, 0.1), (18.0, 0.01), (19.0, 0.002), (20.0, 0.0001)];
    if t <= K[0].0 {
        return K[0].1;
    }
    for w in K.windows(2) {
        if t <= w[1].0 {
            let a = (t - w[0].0) / (w[1].0 - w[0].0);
            return w[0].1 + (w[1].1 - w[0].1) * a;
        }
    }
    K[6].1
}

/// FPlateReverbFast + FSubmixEffectReverb's wet ramp
#[derive(Clone, Debug)]
pub struct PlateReverb {
    settings: PlateSettings,
    early: Early,
    late: Late,
    /// +0x130: the wet volume the last block ended on (< 0 = none yet)
    wet: f32,
    wet_target: f32,
}

impl Default for PlateReverb {
    fn default() -> Self {
        PlateReverb::new()
    }
}

impl PlateReverb {
    /// FSubmixEffectReverb::Init 0x2cc2f20: the plate starts from these settings (0x2cc2f46..0x2cc2f7d)
    pub fn new() -> PlateReverb {
        let mut s = PlateSettings::default();
        s.early.decay = 0.9;
        s.late = LateSettings { late_delay_msec: 0.0, late_gain_db: 0.0, bandwidth: 0.0, diffusion: 0.0, dampening: 0.35, decay: 0.15, density: 0.85 };
        PlateReverb { settings: s, early: Early::new(s.early), late: Late::new(s.late), wet: -1.0, wet_target: 0.0 }
    }

    /// SetSettings 0x2c9a650: a stage that turns off is flushed; the settings are clamped and applied
    pub fn set(&mut self, s: PlateSettings, wet: f32) {
        self.wet_target = wet;
        if s == self.settings {
            return;
        }
        if self.settings.enable_early && !s.enable_early {
            self.early.clear();
        }
        if self.settings.enable_late && !s.enable_late {
            self.late.clear();
        }
        self.settings = s;
        self.early.apply(s.early.clamped());
        self.late.apply(s.late.clamped());
    }

    /// OnProcessAudio 0x2cc9310 for one block of stereo frames: `input` (the reverb submix's summed sends) in,
    /// the wet output added into `out`
    pub fn process(&mut self, input: &[f32], out: &mut [f32]) {
        let start = if self.wet < 0.0 { self.wet_target } else { self.wet };
        self.wet = self.wet_target;
        let s = self.settings;
        if !s.enable_early && !s.enable_late {
            return;
        }
        let mut x = input.to_vec();
        let g = fade_gains(x.len(), start, self.wet_target);
        for (i, v) in x.iter_mut().enumerate() {
            *v *= g(i);
        }
        for (xi, oi) in x.chunks(MAX_BLOCK * 2).zip(out.chunks_mut(MAX_BLOCK * 2)) {
            if s.enable_early {
                for f in 0..xi.len() / 2 {
                    let (l, r) = self.early.tick(xi[2 * f], xi[2 * f + 1]);
                    oi[2 * f] += l;
                    oi[2 * f + 1] += r;
                }
            }
            if s.enable_late {
                self.late.process(xi, oi);
            }
        }
    }
}

/// FReverbSettings (an AudioVolume's Settings, or the world's DefaultReverbSettings: AWorldSettings ctor
/// 0x35762ea bApplyReverb true, ReverbEffect none, Volume 0.5, FadeTime 2)
#[derive(Clone, Debug, PartialEq)]
pub struct ReverbSettings {
    pub apply: bool,
    /// the ReverbEffect asset's properties (None = no effect)
    pub effect: Option<(String, ReverbEffect)>,
    pub volume: f32,
    pub fade_time: f32,
}

impl Default for ReverbSettings {
    fn default() -> Self {
        ReverbSettings { apply: true, effect: None, volume: 0.5, fade_time: 2.0 }
    }
}

/// FAudioEffectsManager's reverb half
#[derive(Clone, Debug)]
pub struct EffectsManager {
    pub settings: ReverbSettings,
    src: (f64, ReverbEffect),
    dst: (f64, ReverbEffect),
    pub current: ReverbEffect,
}

impl Default for EffectsManager {
    fn default() -> Self {
        EffectsManager { settings: ReverbSettings { apply: false, effect: None, volume: 0.0, fade_time: 0.0 }, src: (0.0, ReverbEffect::default()), dst: (0.0, ReverbEffect::default()), current: ReverbEffect::default() }
    }
}

impl EffectsManager {
    /// SetReverbSettings 0x2f205e0 (bForce false): unchanged apply flag, effect and volume -> nothing; else the
    /// source is the current effect at `now`, the destination the asset's values (kept when there is none) at
    /// now + FadeTime with Volume = the settings' (0 without an effect or with bApplyReverb off)
    pub fn set(&mut self, s: &ReverbSettings, now: f64) {
        let same_fx = self.settings.effect.as_ref().map(|e| &e.0) == s.effect.as_ref().map(|e| &e.0);
        if self.settings.apply == s.apply && same_fx && (s.volume - self.settings.volume).abs() <= 1e-8 {
            return;
        }
        self.settings = s.clone();
        self.src = (now, self.current);
        let mut d = s.effect.as_ref().map_or(self.dst.1, |e| e.1);
        d.volume = if s.effect.is_some() && s.apply { s.volume } else { 0.0 };
        self.dst = (now + s.fade_time as f64, d);
    }

    /// Update 0x2f234d0 -> Interpolate 0x2f1a3f0: t = (now - src) / (dst - src) (1 when dst <= src)
    pub fn update(&mut self, now: f64) -> ReverbEffect {
        let span = self.dst.0 - self.src.0;
        let t = if span > 0.0 { ((now - self.src.0) / span) as f32 } else { 1.0 };
        self.current = if t >= 1.0 {
            self.dst.1
        } else if t <= 0.0 {
            self.src.1
        } else {
            ReverbEffect::lerp(&self.src.1, &self.dst.1, t)
        };
        self.current
    }
}

/// a sound's send level to the reverb submix (FMixerSource::UpdateEffects 0x2cdb742..0x2cdb815): only when the
/// source has reverb applied (FSoundSource::SetReverbApplied 0x2efe370: the class's or sound's bReverb, not music);
/// ReverbSendMethod Manual -> clamp01(ManualReverbSendLevel), else alpha = clamp01((distance - DistanceMin) /
/// max(DistanceMax - DistanceMin, 1)) and Linear lerps WetLevelMin..Max, CustomCurve evaluates the curve, both
/// clamped to [0, 1]. Without attenuation (or bEnableReverbSend off) the parse parameters keep their zero ranges
/// (FSoundParseParameters ctor 0x2e1f060), so the level is 0.
pub fn send_level(att: Option<&mh_assets::sound_cue::Attenuation>, class_reverb: bool, is_music: bool, dist_cm: f64) -> f32 {
    if !class_reverb || is_music {
        return 0.0;
    }
    let Some(a) = att.filter(|a| a.enable_reverb_send) else { return 0.0 };
    let c = |x: f64| x.clamp(0.0, 1.0) as f32;
    if a.reverb_send_method == "Manual" {
        return c(a.manual_reverb_send);
    }
    let den = (a.reverb_distance_max - a.reverb_distance_min).max(1.0);
    let alpha = ((dist_cm - a.reverb_distance_min) / den).clamp(0.0, 1.0);
    if a.reverb_send_method == "CustomCurve" {
        c(a.custom_reverb_send_curve.eval(alpha, 0.0))
    } else {
        c(a.reverb_wet_min + (a.reverb_wet_max - a.reverb_wet_min) * alpha)
    }
}

/// the master reverb submix between the voices and the output.
/// Offline (OfflineMix): the voices' sends are summed per block and the reverb runs on them in the same block, as
/// the exe's submix graph does. Device (rodio pulls every voice and this bus as separate sources, in lockstep):
/// voices add their sends into a ring keyed by output frame and the bus source runs the reverb on the frames one
/// BLOCK behind its playback position, so every voice's block covering them has been rendered (UNCONFIRMED: a
/// 1024-frame / 21 ms extra delay of the wet signal on the device path only).
pub struct Bus {
    pub reverb: std::sync::Mutex<PlateReverb>,
    ring: std::sync::Mutex<Vec<f32>>,
    /// the bus source's playback position (output frames)
    pub pos: std::sync::atomic::AtomicU64,
}

const RING: usize = 1 << 15;
const BLOCK: usize = crate::mixer::BLOCK;

impl Default for Bus {
    fn default() -> Self {
        Bus { reverb: std::sync::Mutex::new(PlateReverb::new()), ring: std::sync::Mutex::new(vec![0.0; RING * 2]), pos: std::sync::atomic::AtomicU64::new(0) }
    }
}

impl Bus {
    /// a voice's send block starting at output frame `at`
    pub fn add_send(&self, at: u64, send: &[f32]) {
        let Ok(mut r) = self.ring.lock() else { return };
        for (i, v) in send.iter().enumerate() {
            let f = (at as usize + i / 2) % RING;
            r[f * 2 + i % 2] += *v;
        }
    }
    /// take (and clear) the sends of frames [at, at + n)
    fn take(&self, at: u64, n: usize) -> Vec<f32> {
        let mut out = vec![0.0; n * 2];
        let Ok(mut r) = self.ring.lock() else { return out };
        for (i, o) in out.iter_mut().enumerate() {
            let f = (at as usize + i / 2) % RING;
            *o = std::mem::take(&mut r[f * 2 + i % 2]);
        }
        out
    }
}

/// the bus as a bevy_audio source (device output)
#[derive(bevy::asset::Asset, bevy::reflect::TypePath)]
pub struct BusVoice(pub std::sync::Arc<Bus>);

pub struct BusSource {
    bus: std::sync::Arc<Bus>,
    buf: Vec<f32>,
    idx: usize,
    frame: u64,
}

impl Iterator for BusSource {
    type Item = f32;
    fn next(&mut self) -> Option<f32> {
        if self.idx >= self.buf.len() {
            // the wet output of the sends one block behind
            let input = if self.frame >= BLOCK as u64 { self.bus.take(self.frame - BLOCK as u64, BLOCK) } else { vec![0.0; BLOCK * 2] };
            self.buf = vec![0.0; BLOCK * 2];
            if let Ok(mut r) = self.bus.reverb.lock() {
                r.process(&input, &mut self.buf);
            }
            self.idx = 0;
        }
        if self.idx % 2 == 0 {
            self.bus.pos.store(self.frame, std::sync::atomic::Ordering::Relaxed);
        }
        let s = self.buf[self.idx];
        self.idx += 1;
        if self.idx % 2 == 0 {
            self.frame += 1;
        }
        Some(s)
    }
}

impl rodio::Source for BusSource {
    fn current_span_len(&self) -> Option<usize> {
        None
    }
    fn channels(&self) -> rodio::ChannelCount {
        rodio::ChannelCount::new(2).unwrap()
    }
    fn sample_rate(&self) -> rodio::SampleRate {
        rodio::SampleRate::new(crate::mixer::MIXER_RATE).unwrap()
    }
    fn total_duration(&self) -> Option<std::time::Duration> {
        None
    }
    fn try_seek(&mut self, _pos: std::time::Duration) -> Result<(), rodio::source::SeekError> {
        Err(rodio::source::SeekError::NotSupported { underlying_source: "mh_audio::reverb::BusSource" })
    }
}

impl bevy::audio::Decodable for BusVoice {
    type Decoder = BusSource;
    fn decoder(&self) -> BusSource {
        BusSource { bus: self.0.clone(), buf: vec![], idx: 0, frame: 0 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// an impulse through TowerReverb's settings rings for longer than the pre-delay and decays
    #[test]
    fn tower_impulse_rings_and_decays() {
        let fx = ReverbEffect::from_asset(&serde_json::json!({"ReflectionsDelay": 0.021286, "ReflectionsGain": 0.230571, "DecayTime": 1.5, "LateGain": 1.450476}));
        let mut fx = fx;
        fx.volume = 0.5;
        let (s, wet) = fx.plate_settings();
        assert!(!s.enable_early && s.enable_late);
        let mut r = PlateReverb::new();
        r.set(s, wet);
        let block = 1024;
        let mut energy = vec![];
        for b in 0..96 {
            let mut x = vec![0.0f32; block * 2];
            if b == 0 {
                x[0] = 1.0;
                x[1] = 1.0;
            }
            let mut o = vec![0.0f32; block * 2];
            r.process(&x, &mut o);
            energy.push(o.iter().map(|v| v * v).sum::<f32>());
        }
        let peak = energy.iter().cloned().fold(0.0, f32::max);
        assert!(peak > 0.0);
        assert!(energy[95] < peak * 1e-3, "{energy:?}");
        assert!(energy.iter().all(|e| e.is_finite()));
    }

    #[test]
    fn decay_curve_keys() {
        assert_eq!(decay_curve(0.0), 0.99);
        assert!((decay_curve(1.5) - (0.99 + (0.45 - 0.99) * 0.75)).abs() < 1e-6);
        assert_eq!(decay_curve(30.0), 0.0001);
    }
}

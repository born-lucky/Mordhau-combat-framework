//! SoundCue graphs and SoundAttenuation settings as engine-neutral data, read from the paks (mh-pak export JSON), plus
//! the play-time evaluation (one play of a cue -> the waves with volume / pitch / delay) and UE's distance attenuation.
//! Port of godot/components/ue/ue_sound.gd (`cue`, `_node`, `attenuation_settings`, `gain`, `evaluate`, `_pick`,
//! `_mc_value`); every native default and rule below was read from the shipping exe there (RVA = .text offset in
//! extract/native/publics.tsv + 0x1000), with field names from the exe's UHT property tables:
//!   USoundNodeModulator ctor 0x3450e10: PitchMin@0x48=0.95 PitchMax@0x4c=1.05 VolumeMin@0x50=0.95 VolumeMax@0x54=1.05
//!   USoundNodeModulator::ParseNodes 0x3467ca0: once per active sound v = Max + (Min - Max) * u, u = FRandomStream [0,1)
//!   USoundNodeRandom ctor 0x3450f30: bRandomizeWithoutReplacement = true (bitfield 0x70|4), Weights empty
//!   USoundNodeRandom::ChooseNodeIndex 0x3458280: r = FRand * sum(weights of unused), first child whose running sum > r
//!   USoundNodeMixer::ParseNodes 0x3467b80: every child, Volume *= InputVolume[i]
//!   USoundNodeSwitch::ParseNodes 0x3468690: i = param found ? value + 1 : 0; out of range -> 0
//!   USoundNodeBranch::ParseNodes 0x3465ba0: param unset -> child 2, true -> 0, false -> 1
//!   USoundNodeDelay::ParseNodes 0x34660a0: delay = max(DelayMax + (DelayMin - DelayMax) * u, 0)
//!   USoundNodeModulatorContinuous ctor 0x3450e50 + FModulatorContinuousParams::GetValue 0x3461ef0: Default 1, MinInput 0,
//!     MaxInput 1, MinOutput 0, MaxOutput 1, ParamMode 0 (MPM_Normal / MPM_Abs / MPM_Direct = 0 / 1 / 2)
//!   USoundCue ctor 0x3450830: VolumeMultiplier@0x1c8 = 0.75, PitchMultiplier@0x1cc = 1; USoundWave ctor Volume = Pitch = 1
//!   FBaseAttenuationSettings ctor 0x2ee93a0 / FSoundAttenuationSettings ctor 0x2ca8c60: the `Attenuation` defaults
//!   FBaseAttenuationSettings::Evaluate 0x2ef26a0 (Sphere) + AttenuationEval 0x2eed6f0: `gain`
//! Read for mh-assets r4 (scripts/ue_dis.py on the .text offsets in publics.tsv):
//!   USoundNodeRandom::ParseNodes 0x34683b0: the child is chosen once per active sound (node payload bFirstTime /
//!     NodeIndex); then, without replacement, when NumRandomUsed (+0x68) >= HasBeenUsed.Num() (+0x60): every entry
//!     cleared, HasBeenUsed[NodeIndex] = true, NumRandomUsed = 1 - so the reset happens AFTER the pick that used the
//!     last child, and the next play never repeats it.
//!   USoundNodeRandom::ChooseNodeIndex 0x3458280: total = sum of Weights[i] over i < Weights.Num() (unused only when
//!     without replacement), r = (rand() & 0x7fff) / 32767 * total, the first i < min(ChildNodes.Num(), Weights.Num())
//!     (unused only) with r < running sum; HasBeenUsed[i] = true and NumRandomUsed++ (always); none found -> 0 unmarked.
//!   USoundNodeDistanceCrossFade::ParseNodes 0x3466480: per child {a, b, c, d, v} = FadeIn start / end, FadeOut start
//!     / end, Volume and listener distance x: crossfading not allowed -> v; a <= x <= b -> (b > 0 ? (x - a) / (b - a) :
//!     1) v; c <= x <= d -> (d > 0 ? 1 - (x - c) / (d - c) : 0) v; b <= x <= c -> v; else 0; every child parsed with
//!     Volume *= that.
//!   USoundNodeLooping ctor 0x3450d80, USoundNodeEnveloper ctor 0x3450ce0, USoundNodeDoppler ctor 0x3450ca0: defaults
//!     on the NodeData variants.
//! UNCONFIRMED (ParseNodes / GetDuration not disassembled; UE 4.26 semantics recalled): Looping (0x3467510;
//! GetDuration 0x345f6d0), Concatenator (0x3465c60: children in sequence; GetDuration 0x345f560 = sum), Enveloper
//! (0x3466b50: curves over playback time x a modulation picked once; GetDuration 0x345f660), Doppler (0x3466870),
//! Attenuation (0x3465ab0: the node's settings replace the cue's for its subtree), the Custom attenuation curve and
//! FRichCurve::Eval.
//! Units: UE cm (the Godot port converts to metres; the ratios are the same).

use crate::material::ue_pkg_path;
use mh_pak::Reader;
use serde_json::Value;
use std::collections::HashMap;

/// FRichCurve key (FRuntimeFloatCurve.EditorCurveData.Keys)
#[derive(Debug, Clone, PartialEq)]
pub struct CurveKey {
    /// ERichCurveInterpMode: RCIM_Linear, RCIM_Constant, RCIM_Cubic, RCIM_None
    pub interp: String,
    pub time: f64,
    pub value: f64,
    pub arrive_tangent: f64,
    pub leave_tangent: f64,
}

/// FRichCurve (no weighted tangents: every Mordhau sound curve is RCTWM_WeightedNone)
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Curve {
    pub keys: Vec<CurveKey>,
}

impl Curve {
    /// FRichCurve::Eval, UE 4.26 RichCurve.cpp (UNCONFIRMED: recalled): constant before the first / after the last
    /// key; between keys by the left key's mode: Constant = left value, Linear = lerp, Cubic = Bezier with
    /// P1 = v0 + LeaveTangent0 * dt / 3, P2 = v1 - ArriveTangent1 * dt / 3. Empty curve -> `default`.
    pub fn eval(&self, t: f64, default: f64) -> f64 {
        let k = &self.keys;
        if k.is_empty() {
            return default;
        }
        if t <= k[0].time {
            return k[0].value;
        }
        let last = k.len() - 1;
        if t >= k[last].time {
            return k[last].value;
        }
        let i = k.iter().rposition(|x| x.time <= t).unwrap_or(0);
        let (a, b) = (&k[i], &k[i + 1]);
        let dt = b.time - a.time;
        if dt <= 0.0 {
            return a.value;
        }
        let u = (t - a.time) / dt;
        match a.interp.trim_start_matches("ERichCurveInterpMode::") {
            "RCIM_Constant" => a.value,
            "RCIM_Cubic" => {
                let p1 = a.value + a.leave_tangent * dt / 3.0;
                let p2 = b.value - b.arrive_tangent * dt / 3.0;
                let w = 1.0 - u;
                w * w * w * a.value + 3.0 * w * w * u * p1 + 3.0 * w * u * u * p2 + u * u * u * b.value
            }
            _ => a.value + (b.value - a.value) * u,
        }
    }
}

/// A FRuntimeFloatCurve JSON value ({"EditorCurveData": {"Keys": [...]}})
pub fn curve(v: Option<&Value>) -> Curve {
    let keys = v
        .and_then(|v| v.get("EditorCurveData"))
        .and_then(|c| c.get("Keys"))
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .map(|k| CurveKey {
                    interp: s(k, "InterpMode", "RCIM_Linear").to_string(),
                    time: f(k, "Time", 0.0),
                    value: f(k, "Value", 0.0),
                    arrive_tangent: f(k, "ArriveTangent", 0.0),
                    leave_tangent: f(k, "LeaveTangent", 0.0),
                })
                .collect()
        })
        .unwrap_or_default();
    Curve { keys }
}

/// USoundNodeEnveloper parameters; the curves run over the sound's playback time
#[derive(Debug, Clone, PartialEq)]
pub struct Envelope {
    pub loop_start: f64,
    pub loop_end: f64,
    pub duration_after_loop: f64,
    pub loop_count: i64,
    pub loop_indefinitely: bool,
    pub looping: bool,
    pub volume_curve: Curve,
    pub pitch_curve: Curve,
    pub pitch_min: f64,
    pub pitch_max: f64,
    pub volume_min: f64,
    pub volume_max: f64,
}

/// USoundNodeDoppler parameters
#[derive(Debug, Clone, PartialEq)]
pub struct Doppler {
    pub intensity: f64,
    pub smoothing: bool,
    pub smoothing_speed: f64,
}

/// UE's INDEFINITELY_LOOPING_DURATION (the Duration looping waves serialize, e.g. 10000 in extract/json)
pub const INDEFINITELY_LOOPING_DURATION: f64 = 10000.0;

/// FModulatorContinuousParams
#[derive(Debug, Clone, PartialEq)]
pub struct ContinuousParams {
    pub param: String,
    /// "EModulationParamMode::MPM_Normal" style as stored ("" = the ctor's MPM_Normal)
    pub mode: String,
    pub default: f64,
    pub min_input: f64,
    pub max_input: f64,
    pub min_output: f64,
    pub max_output: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NodeData {
    /// duration = the SoundWave's Duration (s)
    WavePlayer { wave: String, looping: bool, wave_volume: f64, wave_pitch: f64, duration: f64 },
    Modulator { pitch_min: f64, pitch_max: f64, volume_min: f64, volume_max: f64 },
    Random { weights: Vec<f64>, no_repeat: bool },
    Mixer { input_volume: Vec<f64> },
    Switch { param: String },
    Branch { param: String },
    Delay { min: f64, max: f64 },
    ModulatorContinuous { pitch: ContinuousParams, volume: ContinuousParams },
    /// USoundNodeLooping (ctor 0x3450d80: LoopCount@0x48 = 1, bLoopIndefinitely bit0 @0x4c = true)
    Looping { loop_count: i64, indefinitely: bool },
    /// USoundNodeConcatenator: children one after another, InputVolume per child
    Concatenator { input_volume: Vec<f64> },
    /// USoundNodeEnveloper (ctor 0x3450ce0: PitchMin / PitchMax / VolumeMin / VolumeMax @0x180..0x18c = 1, rest 0)
    Enveloper(Envelope),
    /// USoundNodeDoppler (ctor 0x3450ca0: DopplerIntensity@0x48 = 1, bUseSmoothing@0x4c = false,
    /// SmoothingInterpSpeed@0x50 = 5)
    Doppler(Doppler),
    /// USoundNodeDistanceCrossFade: per child [FadeInDistanceStart, FadeInDistanceEnd, FadeOutDistanceStart,
    /// FadeOutDistanceEnd, Volume]
    DistanceCrossFade { inputs: Vec<[f64; 5]> },
    /// USoundNodeAttenuation: the attenuation for its subtree
    Attenuation(Option<Attenuation>),
    /// a node type not ported: plays its first child
    Other,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CueNode {
    /// class name without "SoundNode" ("Random", "WavePlayer", ...)
    pub kind: String,
    pub data: NodeData,
    pub children: Vec<CueNode>,
}

/// FSoundAttenuationSettings with the native defaults filled
#[derive(Debug, Clone, PartialEq)]
pub struct Attenuation {
    /// EAttenuationDistanceModel: Linear, Logarithmic, Inverse, LogReverse, NaturalSound, Custom
    pub algorithm: String,
    /// EAttenuationShape: Sphere, Capsule, Box, Cone
    pub shape: String,
    /// ENaturalSoundFalloffMode: Continues, Silent, Hold
    pub falloff_mode: String,
    /// AttenuationShapeExtents.X: full-volume radius of a sphere (cm)
    pub inner_radius_cm: f64,
    pub falloff_cm: f64,
    pub db_at_max: f64,
    pub attenuate: bool,
    pub spatialize: bool,
    /// OmniRadius (cm): FSoundAttenuationSettings ctor 0x2ca8c60 default 0
    pub omni_radius_cm: f64,
    /// StereoSpread (cm): ctor 0x2ca8cce default 200
    pub stereo_spread_cm: f64,
    pub attenuate_with_lpf: bool,
    pub lpf_radius_min: f64,
    pub lpf_radius_max: f64,
    pub lpf_frequency_at_min: f64,
    pub lpf_frequency_at_max: f64,
    /// AbsorptionMethod (+0xb8): Linear / CustomCurve (ctor 0x2ca8cc3: Linear)
    pub absorption_method: String,
    /// bEnableLogFrequencyScaling (+0xb1 bit 2; ctor 0x2ca8caa: false)
    pub log_frequency_scaling: bool,
    /// CustomLowpassAirAbsorptionCurve (+0xd0)
    pub custom_lpf_curve: Curve,
    /// bEnableOcclusion (+0xb0 bit 5; ctor 0x2ca8c93: false)
    pub enable_occlusion: bool,
    /// OcclusionTraceChannel (+0xb9; ctor 0x2ca8cc3: 3 = ECC_Visibility)
    pub occlusion_trace_channel: String,
    /// OcclusionLowPassFilterFrequency / OcclusionVolumeAttenuation / OcclusionInterpolationTime (ctor 0x2ca8d85:
    /// 20000, 1, 0.1)
    pub occlusion_lpf: f64,
    pub occlusion_volume: f64,
    pub occlusion_interpolation_time: f64,
    /// bEnableReverbSend (+0xb0 bit 7; ctor 0x2ca8c97: true)
    pub enable_reverb_send: bool,
    /// ReverbSendMethod (+0xba): Linear / CustomCurve / Manual
    pub reverb_send_method: String,
    /// ReverbWetLevelMin / Max (ctor 0x2ca8da3: 0.3 / 0.95)
    pub reverb_wet_min: f64,
    pub reverb_wet_max: f64,
    /// ReverbDistanceMin / Max (ctor 0x2ca8dbc: AttenuationShapeExtents.X / X + FalloffDistance)
    pub reverb_distance_min: f64,
    pub reverb_distance_max: f64,
    /// ManualReverbSendLevel (ctor 0x2ca8dd1: 0)
    pub manual_reverb_send: f64,
    pub custom_reverb_send_curve: Curve,
    /// CustomAttenuationCurve (DistanceAlgorithm Custom): gain over d / FalloffDistance
    pub custom_curve: Curve,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SoundCue {
    pub path: String,
    pub volume: f64,
    pub pitch: f64,
    pub attenuation: Option<Attenuation>,
    pub root: Option<CueNode>,
    /// every wave the graph can play (first-seen order)
    pub waves: Vec<String>,
    /// node types followed by their first child (not read in the exe)
    pub unknown: Vec<String>,
}

impl SoundCue {
    pub fn exact(&self) -> bool {
        self.unknown.is_empty()
    }
}

fn f(p: &Value, k: &str, d: f64) -> f64 {
    p.get(k).and_then(Value::as_f64).unwrap_or(d)
}
fn b(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn s<'a>(p: &'a Value, k: &str, d: &'a str) -> &'a str {
    p.get(k).and_then(Value::as_str).unwrap_or(d)
}
fn tail(x: &str) -> String {
    x.split("::").nth(1).unwrap_or("").to_string()
}

/// FSoundAttenuationSettings over the native defaults (ue_sound.gd ATT_DEF / attenuation_settings)
pub fn attenuation_settings(st: &Value) -> Attenuation {
    let ex = st.get("AttenuationShapeExtents");
    Attenuation {
        algorithm: tail(s(st, "DistanceAlgorithm", "EAttenuationDistanceModel::Linear")),
        shape: tail(s(st, "AttenuationShape", "EAttenuationShape::Sphere")),
        falloff_mode: tail(s(st, "FalloffMode", "ENaturalSoundFalloffMode::Continues")),
        inner_radius_cm: ex.map_or(400.0, |e| f(e, "X", 0.0)),
        falloff_cm: f(st, "FalloffDistance", 3600.0),
        db_at_max: f(st, "dBAttenuationAtMax", -60.0),
        attenuate: b(st, "bAttenuate", true),
        spatialize: b(st, "bSpatialize", true),
        omni_radius_cm: f(st, "OmniRadius", 0.0),
        stereo_spread_cm: f(st, "StereoSpread", 200.0),
        attenuate_with_lpf: b(st, "bAttenuateWithLPF", false),
        lpf_radius_min: f(st, "LPFRadiusMin", 3000.0),
        lpf_radius_max: f(st, "LPFRadiusMax", 6000.0),
        lpf_frequency_at_min: f(st, "LPFFrequencyAtMin", 20000.0),
        lpf_frequency_at_max: f(st, "LPFFrequencyAtMax", 20000.0),
        enable_occlusion: b(st, "bEnableOcclusion", false),
        occlusion_trace_channel: s(st, "OcclusionTraceChannel", "ECC_Visibility").to_string(),
        occlusion_lpf: f(st, "OcclusionLowPassFilterFrequency", 20000.0),
        occlusion_volume: f(st, "OcclusionVolumeAttenuation", 1.0),
        occlusion_interpolation_time: f(st, "OcclusionInterpolationTime", 0.1),
        enable_reverb_send: b(st, "bEnableReverbSend", true),
        reverb_send_method: tail(s(st, "ReverbSendMethod", "EReverbSendMethod::Linear")),
        reverb_wet_min: f(st, "ReverbWetLevelMin", 0.3),
        reverb_wet_max: f(st, "ReverbWetLevelMax", 0.95),
        reverb_distance_min: f(st, "ReverbDistanceMin", ex.map_or(400.0, |e| f(e, "X", 0.0))),
        reverb_distance_max: f(st, "ReverbDistanceMax", ex.map_or(400.0, |e| f(e, "X", 0.0)) + f(st, "FalloffDistance", 3600.0)),
        manual_reverb_send: f(st, "ManualReverbSendLevel", 0.0),
        custom_reverb_send_curve: curve(st.get("CustomReverbSendCurve")),
        absorption_method: tail(s(st, "AbsorptionMethod", "EAirAbsorptionMethod::Linear")),
        log_frequency_scaling: b(st, "bEnableLogFrequencyScaling", false),
        custom_lpf_curve: curve(st.get("CustomLowpassAirAbsorptionCurve")),
        custom_curve: curve(st.get("CustomAttenuationCurve")),
    }
}

/// the attenuation low-pass cutoff (Hz) at a distance: FActiveSound::GetAttenuationFrequency 0x2e2af70, called by
/// UpdateAttenuation 0x2e3fb99 when bAttenuateWithLPF (+0xb0 bit 2) is set; the distance is the listener's
/// AttenuationDistance (the distance past the shape's inner radius, as for gain()).
/// - AtMin == AtMax: AtMin; RadiusMin == RadiusMax: d <= RadiusMin ? AtMin : AtMax;
/// - Linear: t = (d - RMin) / (RMax - RMin) clamped (0 when the span is <= 1e-8); log scaling interpolates ln f;
/// - CustomCurve: c = clamp01(curve(t)) between min(AtMin, AtMax) and max(...), likewise linear or in ln f;
/// - the result is clamped to [20, 20000] (0x2e2b1bc).
pub fn attenuation_lpf(a: &Attenuation, dist_cm: f64) -> f64 {
    if !a.attenuate_with_lpf {
        return 20000.0;
    }
    let d = (dist_cm - a.inner_radius_cm).max(0.0) as f32;
    let (fmin, fmax) = (a.lpf_frequency_at_min as f32, a.lpf_frequency_at_max as f32);
    let (rmin, rmax) = (a.lpf_radius_min as f32, a.lpf_radius_max as f32);
    let span = rmax - rmin;
    let lerp_log = |x: f32, y: f32, t: f32| (x.ln() + (y.ln() - x.ln()) * t).exp();
    let r = if fmin == fmax {
        fmin
    } else if rmin == rmax {
        if rmin >= d { fmin } else { fmax }
    } else if a.absorption_method == "CustomCurve" {
        let (lo, hi) = (fmin.min(fmax), fmin.max(fmax));
        let t = ((d - rmin) / span).clamp(0.0, 1.0);
        let c = (a.custom_lpf_curve.eval(t as f64, 0.0) as f32).clamp(0.0, 1.0);
        if a.log_frequency_scaling {
            if c <= 0.0 { lo } else if c >= 1.0 { hi } else { lerp_log(lo, hi, c) }
        } else {
            lo + (hi - lo) * c
        }
    } else if a.log_frequency_scaling {
        if d <= rmin {
            fmin
        } else if d >= rmax {
            fmax
        } else {
            let t = if span.abs() > 1e-8 { (d - rmin) / span } else { 0.0 };
            lerp_log(fmin, fmax, t)
        }
    } else {
        let t = if span.abs() > 1e-8 { (d - rmin) / span } else if d >= rmax { 1.0 } else { 0.0 };
        fmin + (fmax - fmin) * t.clamp(0.0, 1.0)
    };
    if r >= 20.0 { r.min(20000.0) as f64 } else { 20.0 }
}

fn export_of<'a>(ex: &'a [Value], ty: &str) -> Option<&'a Value> {
    ex.iter().find(|e| e.get("Type").and_then(Value::as_str) == Some(ty))
}

/// A SoundAttenuation package's settings
pub fn attenuation(rd: &Reader, pkg: &str) -> Option<Attenuation> {
    let ex = rd.read(crate::material::strip_index(pkg))?;
    let e = export_of(&ex, "SoundAttenuation")?;
    Some(attenuation_settings(e.get("Properties").and_then(|p| p.get("Attenuation")).unwrap_or(&Value::Null)))
}

fn mc(st: &Value) -> ContinuousParams {
    ContinuousParams {
        param: s(st, "ParameterName", "").to_string(),
        mode: s(st, "ParamMode", "").to_string(),
        default: f(st, "Default", 1.0),
        min_input: f(st, "MinInput", 0.0),
        max_input: f(st, "MaxInput", 1.0),
        min_output: f(st, "MinOutput", 0.0),
        max_output: f(st, "MaxOutput", 1.0),
    }
}

/// The SoundCue of a package (ue_sound.gd cue)
pub fn cue(rd: &Reader, path: &str) -> Option<SoundCue> {
    let p = crate::material::strip_index(path).to_string();
    let ex = rd.read(&p)?;
    let c = export_of(&ex, "SoundCue")?;
    let null = Value::Null;
    let pr = c.get("Properties").unwrap_or(&null);
    let mut d = SoundCue {
        path: p,
        volume: f(pr, "VolumeMultiplier", 0.75),
        pitch: f(pr, "PitchMultiplier", 1.0),
        attenuation: None,
        root: None,
        waves: vec![],
        unknown: vec![],
    };
    d.attenuation = if b(pr, "bOverrideAttenuation", false) {
        Some(attenuation_settings(pr.get("AttenuationOverrides").unwrap_or(&null)))
    } else if let Some(a) = pr.get("AttenuationSettings").and_then(|a| a.get("ObjectPath")).and_then(Value::as_str) {
        attenuation(rd, a)
    } else {
        None
    };
    d.root = node(rd, &ex, pr.get("FirstNode"), &mut d, 0);
    Some(d)
}

fn node(rd: &Reader, ex: &[Value], r: Option<&Value>, d: &mut SoundCue, depth: usize) -> Option<CueNode> {
    let on = r?.get("ObjectName")?.as_str()?;
    if depth > 64 {
        return None;
    }
    let nm = on.split('\'').nth(1).unwrap_or("");
    let nm = nm.split(':').nth(1).unwrap_or(nm);
    let null = Value::Null;
    let e = ex.iter().find(|e| e.get("Name").and_then(Value::as_str) == Some(nm)).unwrap_or(&null);
    let kind = s(e, "Type", "").trim_start_matches("SoundNode").to_string();
    let pr = e.get("Properties").unwrap_or(&null);
    let mut children = Vec::new();
    for ch in pr.get("ChildNodes").and_then(Value::as_array).into_iter().flatten() {
        // a null child keeps its slot (an empty node, as ue_sound.gd's {} child)
        children.push(node(rd, ex, Some(ch), d, depth + 1).unwrap_or(CueNode { kind: String::new(), data: NodeData::Other, children: vec![] }));
    }
    let data = match kind.as_str() {
        "WavePlayer" => {
            let mut w = crate::material::strip_index(e.get("SoundWave").and_then(|w| w.get("ObjectPath")).and_then(Value::as_str).unwrap_or("")).to_string();
            if w.is_empty() {
                w = ue_pkg_path(pr.get("SoundWaveAssetPtr").unwrap_or(&null));
            }
            if !d.waves.contains(&w) {
                d.waves.push(w.clone());
            }
            let wp = rd.read(&w).and_then(|x| export_of(&x, "SoundWave").and_then(|e| e.get("Properties").cloned())).unwrap_or(Value::Null);
            NodeData::WavePlayer {
                looping: b(pr, "bLooping", false),
                wave_volume: f(&wp, "Volume", 1.0),
                wave_pitch: f(&wp, "Pitch", 1.0),
                duration: f(&wp, "Duration", 0.0),
                wave: w,
            }
        }
        "Modulator" => NodeData::Modulator {
            pitch_min: f(pr, "PitchMin", 0.95),
            pitch_max: f(pr, "PitchMax", 1.05),
            volume_min: f(pr, "VolumeMin", 0.95),
            volume_max: f(pr, "VolumeMax", 1.05),
        },
        "Random" => NodeData::Random {
            weights: pr.get("Weights").and_then(Value::as_array).map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect()).unwrap_or_default(),
            no_repeat: b(pr, "bRandomizeWithoutReplacement", true),
        },
        "Mixer" => NodeData::Mixer {
            input_volume: pr.get("InputVolume").and_then(Value::as_array).map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect()).unwrap_or_default(),
        },
        "Switch" => NodeData::Switch { param: s(pr, "IntParameterName", "").into() },
        "Branch" => NodeData::Branch { param: s(pr, "BoolParameterName", "").into() },
        "Delay" => NodeData::Delay { min: f(pr, "DelayMin", 0.0), max: f(pr, "DelayMax", 0.0) },
        "ModulatorContinuous" => NodeData::ModulatorContinuous {
            pitch: mc(pr.get("PitchModulationParams").unwrap_or(&null)),
            volume: mc(pr.get("VolumeModulationParams").unwrap_or(&null)),
        },
        "Looping" => NodeData::Looping {
            loop_count: pr.get("LoopCount").and_then(Value::as_i64).unwrap_or(1),
            indefinitely: b(pr, "bLoopIndefinitely", true),
        },
        "Concatenator" => NodeData::Concatenator {
            input_volume: pr.get("InputVolume").and_then(Value::as_array).map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0)).collect()).unwrap_or_default(),
        },
        "Enveloper" => NodeData::Enveloper(Envelope {
            loop_start: f(pr, "LoopStart", 0.0),
            loop_end: f(pr, "LoopEnd", 0.0),
            duration_after_loop: f(pr, "DurationAfterLoop", 0.0),
            loop_count: pr.get("LoopCount").and_then(Value::as_i64).unwrap_or(0),
            loop_indefinitely: b(pr, "bLoopIndefinitely", false),
            looping: b(pr, "bLoop", false),
            volume_curve: curve(pr.get("VolumeCurve")),
            pitch_curve: curve(pr.get("PitchCurve")),
            pitch_min: f(pr, "PitchMin", 1.0),
            pitch_max: f(pr, "PitchMax", 1.0),
            volume_min: f(pr, "VolumeMin", 1.0),
            volume_max: f(pr, "VolumeMax", 1.0),
        }),
        "Doppler" => NodeData::Doppler(Doppler {
            intensity: f(pr, "DopplerIntensity", 1.0),
            smoothing: b(pr, "bUseSmoothing", false),
            smoothing_speed: f(pr, "SmoothingInterpSpeed", 5.0),
        }),
        "DistanceCrossFade" => NodeData::DistanceCrossFade {
            inputs: pr
                .get("CrossFadeInput")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|x| [f(x, "FadeInDistanceStart", 0.0), f(x, "FadeInDistanceEnd", 0.0), f(x, "FadeOutDistanceStart", 0.0),
                            f(x, "FadeOutDistanceEnd", 0.0), f(x, "Volume", 1.0)])
                        .collect()
                })
                .unwrap_or_default(),
        },
        "Attenuation" => NodeData::Attenuation(if b(pr, "bOverrideAttenuation", false) {
            Some(attenuation_settings(pr.get("AttenuationOverrides").unwrap_or(&null)))
        } else {
            pr.get("AttenuationSettings").and_then(|a| a.get("ObjectPath")).and_then(Value::as_str).and_then(|a| attenuation(rd, a))
        }),
        _ => {
            if !d.unknown.contains(&kind) {
                d.unknown.push(kind.clone());
            }
            NodeData::Other
        }
    };
    Some(CueNode { kind, data, children })
}

/// UE AttenuationEval for a sphere at `dist_cm` from the source: linear gain in [0, 1] (ue_sound.gd gain;
/// Evaluate 0x2ef26a0, AttenuationEval 0x2eed6f0). Custom (FRichCurve) is not ported: 1.
pub fn gain(a: Option<&Attenuation>, dist_cm: f64) -> f64 {
    let Some(a) = a.filter(|a| a.attenuate) else { return 1.0 };
    let d = (dist_cm - a.inner_radius_cm).max(0.0);
    let f = a.falloff_cm.max(1.0);
    let r = match a.algorithm.as_str() {
        "Linear" => 1.0 - d / f,
        "Logarithmic" => -0.5 * (d.max(1e-4) / f).ln(),
        "Inverse" => 0.02 / (d.max(1e-4) / f),
        "LogReverse" => {
            if d > f { 0.0 } else { 1.0 + 0.5 * (1.0 - d / f).max(1e-4).ln() }
        }
        "NaturalSound" => {
            let mut al = d / f;
            if a.falloff_mode == "Silent" && al >= 1.0 {
                return 0.0;
            }
            if a.falloff_mode != "Continues" {
                al = al.clamp(0.0, 1.0);
            }
            10f64.powf(al * a.db_at_max / 20.0)
        }
        // Custom: CustomAttenuationCurve at d / f (AttenuationEval 0x2eed6f0 Custom branch, UNCONFIRMED: not read)
        "Custom" => a.custom_curve.eval(d / f, 1.0),
        _ => 1.0,
    };
    r.clamp(0.0, 1.0)
}

/// FModulatorContinuousParams::GetValue 0x3461ef0
pub fn continuous_value(m: &ContinuousParams, params: &HashMap<String, f64>) -> f64 {
    let mut v = if m.param.is_empty() { m.default } else { params.get(&m.param).copied().unwrap_or(m.default) };
    if m.mode.ends_with("Direct") {
        return v;
    }
    if m.mode.ends_with("Abs") {
        v = v.abs();
    }
    let sl = if m.max_input > m.min_input { (m.max_output - m.min_output) / (m.max_input - m.min_input) } else { 0.0 };
    (v.clamp(m.min_input, m.max_input.max(m.min_input)) - m.min_input) * sl + m.min_output
}

/// One wave to start for a play of a cue
#[derive(Debug, Clone, PartialEq)]
pub struct Play {
    pub wave: String,
    pub volume: f64,
    pub pitch: f64,
    /// seconds after the cue starts (Delay nodes, Concatenator order)
    pub delay: f64,
    /// loops forever (the wave player's bLooping, or an indefinite Looping node)
    pub looping: bool,
    /// a Looping node's LoopCount when not indefinite
    pub loop_count: Option<i64>,
    /// Enveloper over this wave: the curves at playback time x the (volume, pitch) modulation picked for this play
    pub envelope: Option<(Envelope, f64, f64)>,
    /// Doppler pitch shift (`doppler_pitch`) for this wave
    pub doppler: Option<Doppler>,
    /// an Attenuation node's settings replacing the cue's for this wave
    pub attenuation: Option<Attenuation>,
}

impl Play {
    /// Volume and pitch multipliers of the envelope at playback time t (s); (1, 1) without one
    pub fn envelope_at(&self, t: f64) -> (f64, f64) {
        match &self.envelope {
            None => (1.0, 1.0),
            Some((e, vm, pm)) => {
                let mut tt = t;
                if e.looping && e.loop_end > e.loop_start && tt > e.loop_end {
                    let span = e.loop_end - e.loop_start;
                    let loops = ((tt - e.loop_end) / span).floor() as i64 + 1;
                    if e.loop_indefinitely || loops <= e.loop_count {
                        tt = e.loop_start + (tt - e.loop_end) % span;
                    } else {
                        tt -= span * e.loop_count as f64;
                    }
                }
                (e.volume_curve.eval(tt, 1.0) * vm, e.pitch_curve.eval(tt, 1.0) * pm)
            }
        }
    }
}

/// Doppler pitch multiplier for a source moving at `src_vel` and a listener at `lis_vel` (cm/s), positions in cm:
/// ((SpeedOfSound + listener speed away) / (SpeedOfSound - source speed toward) - 1) x Intensity + 1, SpeedOfSound =
/// 343 m/s (UE 4.26 USoundNodeDoppler::GetDopplerPitchMultiplier; UNCONFIRMED: recalled, not read from the exe)
pub fn doppler_pitch(d: &Doppler, src: [f64; 3], src_vel: [f64; 3], lis: [f64; 3], lis_vel: [f64; 3]) -> f64 {
    const SPEED_OF_SOUND: f64 = 34300.0;
    let dir = [lis[0] - src[0], lis[1] - src[1], lis[2] - src[2]];
    let l = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    if l <= 1e-8 {
        return 1.0;
    }
    let n = dir.map(|x| x / l);
    let sv = src_vel[0] * n[0] + src_vel[1] * n[1] + src_vel[2] * n[2];
    let lv = lis_vel[0] * n[0] + lis_vel[1] * n[1] + lis_vel[2] * n[2];
    let m = ((SPEED_OF_SOUND + lv) / (SPEED_OF_SOUND - sv)).abs();
    (m - 1.0) * d.intensity + 1.0
}

/// Per-cue play state: the HasBeenUsed set and NumRandomUsed of each Random node (by node visit order). In UE these
/// live on the node object, shared by every active sound of the cue.
#[derive(Debug, Clone, Default)]
pub struct CueState {
    used: HashMap<usize, (Vec<bool>, usize)>,
}

/// Inputs of one play beyond the graph
#[derive(Debug, Clone, Default)]
pub struct EvalCtx {
    /// Switch / Branch / ModulatorContinuous parameters (Branch: 0 = false)
    pub params: HashMap<String, f64>,
    /// listener distance (cm) for DistanceCrossFade; None = crossfading not allowed (each input at its Volume)
    pub distance_cm: Option<f64>,
}

#[derive(Clone)]
struct Mods {
    vol: f64,
    pit: f64,
    dly: f64,
    looping: bool,
    loop_count: Option<i64>,
    envelope: Option<(Envelope, f64, f64)>,
    doppler: Option<Doppler>,
    att: Option<Attenuation>,
}

/// One play of the cue, like UE's ParseNodes (ue_sound.gd evaluate + the r4 nodes). `rng` returns uniform [0, 1).
pub fn evaluate(c: &SoundCue, params: &HashMap<String, f64>, rng: &mut dyn FnMut() -> f64, st: &mut CueState) -> Vec<Play> {
    evaluate_ctx(c, &EvalCtx { params: params.clone(), distance_cm: None }, rng, st)
}

pub fn evaluate_ctx(c: &SoundCue, ctx: &EvalCtx, rng: &mut dyn FnMut() -> f64, st: &mut CueState) -> Vec<Play> {
    let mut out = Vec::new();
    let mut id = 0;
    if let Some(r) = &c.root {
        let m = Mods { vol: c.volume, pit: c.pitch, dly: 0.0, looping: false, loop_count: None, envelope: None, doppler: None, att: None };
        eval(r, ctx, rng, st, &mut id, m, &mut out);
    }
    out
}

/// GetDuration of a subtree (s): WavePlayer = Duration (looping -> INDEFINITELY_LOOPING_DURATION); Looping =
/// indefinite or child x LoopCount; Concatenator = sum; Delay = DelayMax + child; Enveloper with indefinite loop =
/// indefinite; otherwise the longest child (USoundNode::GetDuration). UNCONFIRMED beyond the WavePlayer rule.
pub fn duration(n: &CueNode) -> f64 {
    let longest = || n.children.iter().map(duration).fold(0.0, f64::max);
    match &n.data {
        NodeData::WavePlayer { looping, duration, .. } => {
            if *looping { INDEFINITELY_LOOPING_DURATION } else { *duration }
        }
        NodeData::Looping { loop_count, indefinitely } => {
            if *indefinitely { INDEFINITELY_LOOPING_DURATION } else { (longest() * *loop_count as f64).min(INDEFINITELY_LOOPING_DURATION) }
        }
        NodeData::Concatenator { .. } => n.children.iter().map(duration).sum::<f64>().min(INDEFINITELY_LOOPING_DURATION),
        NodeData::Delay { max, .. } => max + longest(),
        NodeData::Enveloper(e) if e.looping && e.loop_indefinitely => INDEFINITELY_LOOPING_DURATION,
        _ => longest(),
    }
}

fn eval(n: &CueNode, ctx: &EvalCtx, rng: &mut dyn FnMut() -> f64, st: &mut CueState, id: &mut usize, m: Mods, out: &mut Vec<Play>) {
    let me = *id;
    *id += 1;
    let ch = &n.children;
    let params = &ctx.params;
    match &n.data {
        NodeData::WavePlayer { wave, looping, wave_volume, wave_pitch, .. } => {
            if !wave.is_empty() {
                out.push(Play {
                    wave: wave.clone(),
                    volume: m.vol * wave_volume,
                    pitch: m.pit * wave_pitch,
                    delay: m.dly,
                    looping: *looping || m.looping,
                    loop_count: m.loop_count,
                    envelope: m.envelope.clone(),
                    doppler: m.doppler.clone(),
                    attenuation: m.att.clone(),
                });
            }
        }
        NodeData::Modulator { pitch_min, pitch_max, volume_min, volume_max } => {
            let v = volume_max + (volume_min - volume_max) * rng();
            let p = pitch_max + (pitch_min - pitch_max) * rng();
            for c in ch {
                eval(c, ctx, rng, st, id, Mods { vol: m.vol * v, pit: m.pit * p, ..m.clone() }, out);
            }
        }
        NodeData::Random { weights, no_repeat } => {
            if !ch.is_empty() {
                let (used, num_used) = st.used.entry(me).or_insert_with(|| (vec![false; ch.len()], 0));
                // ChooseNodeIndex 0x3458280
                let nw = weights.len().min(used.len());
                let tot: f64 = (0..nw).filter(|&i| !(*no_repeat && used[i])).map(|i| weights[i]).sum();
                let r = rng() * tot;
                let mut acc = 0.0;
                let mut pick = None;
                for i in 0..ch.len().min(nw) {
                    if *no_repeat && used[i] {
                        continue;
                    }
                    acc += weights[i];
                    if r < acc {
                        pick = Some(i);
                        break;
                    }
                }
                let idx = match pick {
                    Some(i) => {
                        used[i] = true;
                        *num_used += 1;
                        i
                    }
                    None => 0,
                };
                // ParseNodes 0x34683b0: reset after the pick that used every child
                if *no_repeat && !used.is_empty() && *num_used >= used.len() {
                    used.fill(false);
                    used[idx] = true;
                    *num_used = 1;
                }
                eval(&ch[idx], ctx, rng, st, id, m, out);
            }
        }
        NodeData::Mixer { input_volume } => {
            for (i, c) in ch.iter().enumerate() {
                eval(c, ctx, rng, st, id, Mods { vol: m.vol * input_volume.get(i).copied().unwrap_or(1.0), ..m.clone() }, out);
            }
        }
        NodeData::Switch { param } => {
            let mut i = params.get(param).map_or(0, |v| *v as i64 + 1);
            if i < 0 || i as usize >= ch.len() {
                i = 0;
            }
            if let Some(c) = ch.get(i as usize) {
                eval(c, ctx, rng, st, id, m, out);
            }
        }
        NodeData::Branch { param } => {
            let i = params.get(param).map_or(2, |v| if *v != 0.0 { 0 } else { 1 });
            if let Some(c) = ch.get(i) {
                eval(c, ctx, rng, st, id, m, out);
            }
        }
        NodeData::Delay { min, max } => {
            let t = (max + (min - max) * rng()).max(0.0);
            for c in ch {
                eval(c, ctx, rng, st, id, Mods { dly: m.dly + t, ..m.clone() }, out);
            }
        }
        NodeData::ModulatorContinuous { pitch, volume } => {
            let vv = continuous_value(volume, params);
            let pp = continuous_value(pitch, params);
            for c in ch {
                eval(c, ctx, rng, st, id, Mods { vol: m.vol * vv, pit: m.pit * pp, ..m.clone() }, out);
            }
        }
        NodeData::Looping { loop_count, indefinitely } => {
            for c in ch {
                let mm = if *indefinitely { Mods { looping: true, ..m.clone() } } else { Mods { loop_count: Some(*loop_count), ..m.clone() } };
                eval(c, ctx, rng, st, id, mm, out);
            }
        }
        NodeData::Concatenator { input_volume } => {
            let mut t = 0.0;
            for (i, c) in ch.iter().enumerate() {
                let mm = Mods { vol: m.vol * input_volume.get(i).copied().unwrap_or(1.0), dly: m.dly + t, ..m.clone() };
                eval(c, ctx, rng, st, id, mm, out);
                t += duration(c);
            }
        }
        NodeData::Enveloper(e) => {
            let vm = e.volume_max + (e.volume_min - e.volume_max) * rng();
            let pm = e.pitch_max + (e.pitch_min - e.pitch_max) * rng();
            for c in ch {
                eval(c, ctx, rng, st, id, Mods { envelope: Some((e.clone(), vm, pm)), ..m.clone() }, out);
            }
        }
        NodeData::Doppler(d) => {
            for c in ch {
                eval(c, ctx, rng, st, id, Mods { doppler: Some(d.clone()), ..m.clone() }, out);
            }
        }
        NodeData::DistanceCrossFade { inputs } => {
            for (i, c) in ch.iter().enumerate() {
                let Some(&[a, b, cc, d, v]) = inputs.get(i) else { continue };
                let g = match ctx.distance_cm {
                    None => v,
                    // as the exe: b > 0 is the only guard, so x = a = b > 0 gives 0 / 0 = NaN there too
                    Some(x) if x >= a && x <= b => if b > 0.0 { (x - a) / (b - a) * v } else { v },
                    Some(x) if x >= cc && x <= d => if d > 0.0 { (1.0 - (x - cc) / (d - cc)) * v } else { 0.0 },
                    Some(x) if x >= b && x <= cc => v,
                    _ => 0.0,
                };
                eval(c, ctx, rng, st, id, Mods { vol: m.vol * g, ..m.clone() }, out);
            }
        }
        NodeData::Attenuation(a) => {
            for c in ch {
                eval(c, ctx, rng, st, id, Mods { att: a.clone().or_else(|| m.att.clone()), ..m.clone() }, out);
            }
        }
        NodeData::Other => {
            if let Some(c) = ch.first() {
                eval(c, ctx, rng, st, id, m, out);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wave(w: &str) -> CueNode {
        CueNode { kind: "WavePlayer".into(), data: NodeData::WavePlayer { wave: w.into(), looping: false, wave_volume: 1.0, wave_pitch: 1.0, duration: 2.0 }, children: vec![] }
    }

    #[test]
    fn modulator_random_without_replacement() {
        let rnd = CueNode { kind: "Random".into(), data: NodeData::Random { weights: vec![1.0, 1.0], no_repeat: true }, children: vec![wave("a"), wave("b")] };
        let m = CueNode { kind: "Modulator".into(), data: NodeData::Modulator { pitch_min: 0.9, pitch_max: 1.1, volume_min: 0.5, volume_max: 1.0 }, children: vec![rnd] };
        let c = SoundCue { path: "x".into(), volume: 0.75, pitch: 1.0, attenuation: None, root: Some(m), waves: vec![], unknown: vec![] };
        let mut st = CueState::default();
        // u = 0 -> v = Max: volume 0.75, pitch 1.1; random r = 0 -> first unused
        let mut z = || 0.0;
        let p1 = evaluate(&c, &HashMap::new(), &mut z, &mut st);
        assert_eq!(p1[0].wave, "a");
        assert!((p1[0].volume - 0.75).abs() < 1e-12 && (p1[0].pitch - 1.1).abs() < 1e-12);
        let p2 = evaluate(&c, &HashMap::new(), &mut z, &mut st);
        assert_eq!(p2[0].wave, "b");
        // the pick of "b" used every child: reset to {b} (ParseNodes 0x34683b0), so the next play is "a" again and
        // the one after that "b" (never the just-played child twice)
        let p3 = evaluate(&c, &HashMap::new(), &mut z, &mut st);
        assert_eq!(p3[0].wave, "a");
        let p4 = evaluate(&c, &HashMap::new(), &mut z, &mut st);
        assert_eq!(p4[0].wave, "b");
    }

    #[test]
    fn natural_sound_and_linear_gain() {
        let mut a = attenuation_settings(&Value::Null);
        assert_eq!((a.algorithm.as_str(), a.inner_radius_cm, a.falloff_cm), ("Linear", 400.0, 3600.0));
        assert_eq!(gain(Some(&a), 400.0), 1.0);
        assert!((gain(Some(&a), 2200.0) - 0.5).abs() < 1e-12);
        a.algorithm = "NaturalSound".into();
        // a = 1 -> 10^(-60/20) = 0.001
        assert!((gain(Some(&a), 4000.0) - 0.001).abs() < 1e-12);
        a.falloff_mode = "Silent".into();
        assert_eq!(gain(Some(&a), 4000.0), 0.0);
    }

    #[test]
    fn continuous_modes() {
        let m = ContinuousParams { param: "Speed".into(), mode: String::new(), default: 1.0, min_input: 0.0, max_input: 10.0, min_output: 0.5, max_output: 1.5 };
        let p: HashMap<String, f64> = [("Speed".to_string(), 5.0)].into();
        assert!((continuous_value(&m, &p) - 1.0).abs() < 1e-12);
        let mut d = m.clone();
        d.mode = "EModulationParamMode::MPM_Direct".into();
        assert_eq!(continuous_value(&d, &p), 5.0);
    }

    #[test]
    fn crossfade_concatenator_and_curves() {
        let xf = CueNode { kind: "DistanceCrossFade".into(), data: NodeData::DistanceCrossFade {
            inputs: vec![[0.0, 0.0, 1000.0, 3000.0, 1.0], [0.0, 0.0, 500.0, 1000.0, 1.0]] }, children: vec![wave("near"), wave("far")] };
        let c = SoundCue { path: "x".into(), volume: 1.0, pitch: 1.0, attenuation: None, root: Some(xf), waves: vec![], unknown: vec![] };
        let ctx = EvalCtx { params: HashMap::new(), distance_cm: Some(2000.0) };
        let p = evaluate_ctx(&c, &ctx, &mut || 0.0, &mut CueState::default());
        assert!((p[0].volume - 0.5).abs() < 1e-12 && p[1].volume == 0.0);
        let cat = CueNode { kind: "Concatenator".into(), data: NodeData::Concatenator { input_volume: vec![1.0, 0.5] }, children: vec![wave("a"), wave("b")] };
        let c = SoundCue { root: Some(cat), ..c };
        let p = evaluate(&c, &HashMap::new(), &mut || 0.0, &mut CueState::default());
        assert_eq!((p[1].delay, p[1].volume), (2.0, 0.5));
        let cv = Curve { keys: vec![
            CurveKey { interp: "RCIM_Linear".into(), time: 0.0, value: 0.0, arrive_tangent: 0.0, leave_tangent: 0.0 },
            CurveKey { interp: "RCIM_Cubic".into(), time: 1.0, value: 1.0, arrive_tangent: 0.0, leave_tangent: 0.0 },
            CurveKey { interp: "RCIM_Linear".into(), time: 2.0, value: 0.0, arrive_tangent: 0.0, leave_tangent: 0.0 },
        ] };
        assert_eq!(cv.eval(0.5, 9.0), 0.5);
        assert_eq!(cv.eval(1.5, 9.0), 0.5); // symmetric flat-tangent Bezier at u = 0.5
        assert_eq!(cv.eval(5.0, 9.0), 0.0);
        let d = Doppler { intensity: 1.0, smoothing: false, smoothing_speed: 5.0 };
        assert!(doppler_pitch(&d, [0.0; 3], [3430.0, 0.0, 0.0], [100.0, 0.0, 0.0], [0.0; 3]) > 1.0);
    }
}

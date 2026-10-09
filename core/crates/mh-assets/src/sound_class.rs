//! Sound routing data: SoundClass trees, SoundMix class adjusters, SoundConcurrency groups and the per-sound
//! routing (class / concurrency / submix / MaxDistance) of a SoundCue or SoundWave, from the paks.
//!
//! Defaults (properties a package does not store) come from the shipped exe's constructors:
//! - FSoundClassProperties ctor 0x344f720: Volume 1, Pitch 1, LowPassFilterFrequency 20000.
//! - USoundConcurrency ctor 0x34507e0 (FSoundConcurrencySettings at +0x28): MaxCount 16, ResolutionRule 3
//!   (StopFarthestThenOldest), RetriggerTime 0, VolumeScale 1, VolumeScaleAttackTime 0.01, VolumeScaleReleaseTime 0.5,
//!   VoiceStealReleaseTime 0, bLimitToOwner false, bVolumeScaleCanRelease false.
//! - USoundMix ctor 0x34509e0: InitialDelay 0, FadeInTime 0.2, Duration -1, FadeOutTime 0.2.
//! - The default class: DefaultEngine.ini [/Script/Engine.AudioSettings] DefaultSoundClassName = MasterSoundClass.
//!
//! Class property propagation (`ClassTree::effective`):
//! - FAudioDevice::RecurseIntoSoundClasses 0x2efa9e0: from the root down, child Volume *= parent Volume, child Pitch
//!   *= parent Pitch (and the UI / music flags are inherited, 0x2efaaa5).
//! - FAudioDevice::ApplyClassAdjusters 0x2eec460: per mix effect, factor = InterpValue x adjuster + 1 - InterpValue
//!   (0x2eec5a9), multiplied into the class's Volume / Pitch, and into every descendant with bApplyToChildren.
//! The mix's InterpValue over its fade-in / fade-out (FSoundMixState) is not read: linear in time (UNCONFIRMED).

use mh_pak::Reader;
use serde_json::Value;
use std::collections::HashMap;

pub const DEFAULT_CLASS: &str = "Mordhau/Content/Mordhau/Audio/Other/MasterSoundClass";

fn f(p: &Value, k: &str, d: f64) -> f64 {
    p.get(k).and_then(Value::as_f64).unwrap_or(d)
}
fn b(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn obj(v: Option<&Value>) -> String {
    v.map(|v| crate::material::strip_index(&crate::material::ue_pkg_path(v)).to_string()).unwrap_or_default()
}
fn export<'a>(ex: &'a [Value], ty: &str) -> Option<&'a Value> {
    ex.iter().find(|e| e.get("Type").and_then(Value::as_str) == Some(ty))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClassProps {
    pub volume: f64,
    pub pitch: f64,
    pub lpf: f64,
    pub is_ui: bool,
    pub is_music: bool,
    /// bReverb (+0x24 bit 6; FSoundClassProperties ctor 0x344f72e: true); not inherited by child classes
    pub reverb: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Passive {
    pub mix: String,
    pub min_volume: f64,
    pub max_volume: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SoundClass {
    pub path: String,
    pub props: ClassProps,
    pub parent: String,
    pub children: Vec<String>,
    pub passive: Vec<Passive>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClassEffect {
    pub class: String,
    pub volume: f64,
    pub pitch: f64,
    pub lpf: f64,
    pub apply_to_children: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SoundMix {
    pub path: String,
    pub effects: Vec<ClassEffect>,
    pub initial_delay: f64,
    pub fade_in: f64,
    pub fade_out: f64,
    /// -1 = until popped
    pub duration: f64,
}

/// EMaxConcurrentResolutionRule
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rule {
    PreventNew,
    StopOldest,
    StopFarthestThenPreventNew,
    StopFarthestThenOldest,
    StopLowestPriority,
    StopQuietest,
    StopLowestPriorityThenPreventNew,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Concurrency {
    pub path: String,
    pub max_count: i64,
    pub limit_to_owner: bool,
    pub rule: Rule,
    pub retrigger_time: f64,
    pub volume_scale: f64,
    pub volume_scale_attack: f64,
    pub volume_scale_can_release: bool,
    pub volume_scale_release: f64,
    pub voice_steal_release: f64,
}

pub fn class(rd: &Reader, path: &str) -> Option<SoundClass> {
    let ex = rd.read(crate::material::strip_index(path))?;
    let e = export(&ex, "SoundClass")?;
    let null = Value::Null;
    let p = e.get("Properties").unwrap_or(&null);
    let pp = p.get("Properties").unwrap_or(&null);
    Some(SoundClass {
        path: crate::material::strip_index(path).to_string(),
        props: ClassProps {
            volume: f(pp, "Volume", 1.0),
            pitch: f(pp, "Pitch", 1.0),
            lpf: f(pp, "LowPassFilterFrequency", 20000.0),
            is_ui: b(pp, "bIsUISound", false),
            is_music: b(pp, "bIsMusic", false),
            reverb: b(pp, "bReverb", true),
        },
        parent: obj(p.get("ParentClass")),
        children: p.get("ChildClasses").and_then(Value::as_array).map(|a| a.iter().map(|c| obj(Some(c))).collect()).unwrap_or_default(),
        passive: p
            .get("PassiveSoundMixModifiers")
            .and_then(Value::as_array)
            .map(|a| a.iter().map(|m| Passive { mix: obj(m.get("SoundMix")), min_volume: f(m, "MinVolumeThreshold", 0.0), max_volume: f(m, "MaxVolumeThreshold", 10.0) }).collect())
            .unwrap_or_default(),
    })
}

pub fn mix(rd: &Reader, path: &str) -> Option<SoundMix> {
    let ex = rd.read(crate::material::strip_index(path))?;
    let e = export(&ex, "SoundMix")?;
    let null = Value::Null;
    let p = e.get("Properties").unwrap_or(&null);
    Some(SoundMix {
        path: crate::material::strip_index(path).to_string(),
        effects: p
            .get("SoundClassEffects")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|x| ClassEffect {
                        class: obj(x.get("SoundClassObject")),
                        volume: f(x, "VolumeAdjuster", 1.0),
                        pitch: f(x, "PitchAdjuster", 1.0),
                        lpf: f(x, "LowPassFilterFrequency", 20000.0),
                        apply_to_children: b(x, "bApplyToChildren", false),
                    })
                    .collect()
            })
            .unwrap_or_default(),
        initial_delay: f(p, "InitialDelay", 0.0),
        fade_in: f(p, "FadeInTime", 0.2),
        fade_out: f(p, "FadeOutTime", 0.2),
        duration: f(p, "Duration", -1.0),
    })
}

pub fn concurrency(rd: &Reader, path: &str) -> Option<Concurrency> {
    let ex = rd.read(crate::material::strip_index(path))?;
    let e = export(&ex, "SoundConcurrency")?;
    let null = Value::Null;
    let c = e.get("Properties").and_then(|p| p.get("Concurrency")).unwrap_or(&null);
    Some(settings(crate::material::strip_index(path), c))
}

/// FSoundConcurrencySettings over the ctor defaults (also a cue's ConcurrencyOverrides)
pub fn settings(path: &str, c: &Value) -> Concurrency {
    let rule = match c.get("ResolutionRule").and_then(Value::as_str).unwrap_or("").rsplit("::").next().unwrap_or("") {
        "PreventNew" => Rule::PreventNew,
        "StopOldest" => Rule::StopOldest,
        "StopFarthestThenPreventNew" => Rule::StopFarthestThenPreventNew,
        "StopLowestPriority" => Rule::StopLowestPriority,
        "StopQuietest" => Rule::StopQuietest,
        "StopLowestPriorityThenPreventNew" => Rule::StopLowestPriorityThenPreventNew,
        _ => Rule::StopFarthestThenOldest,
    };
    Concurrency {
        path: path.to_string(),
        max_count: f(c, "MaxCount", 16.0) as i64,
        limit_to_owner: b(c, "bLimitToOwner", false),
        rule,
        retrigger_time: f(c, "RetriggerTime", 0.0),
        volume_scale: f(c, "VolumeScale", 1.0),
        volume_scale_attack: f(c, "VolumeScaleAttackTime", 0.01),
        volume_scale_can_release: b(c, "bVolumeScaleCanRelease", false),
        volume_scale_release: f(c, "VolumeScaleReleaseTime", 0.5),
        voice_steal_release: f(c, "VoiceStealReleaseTime", 0.0),
    }
}

/// what a sound routes through
#[derive(Debug, Clone, PartialEq)]
pub struct Routing {
    /// the SoundClassObject (empty = the default class)
    pub class: String,
    pub concurrency: Vec<Concurrency>,
    pub max_distance: Option<f64>,
    pub submix: String,
}

/// a SoundCue's or SoundWave's routing (USoundBase: SoundClassObject, ConcurrencySet / bOverrideConcurrency +
/// ConcurrencyOverrides, MaxDistance, SoundSubmixObject)
pub fn routing(rd: &Reader, path: &str) -> Option<Routing> {
    let ex = rd.read(crate::material::strip_index(path))?;
    let e = export(&ex, "SoundCue").or_else(|| export(&ex, "SoundWave"))?;
    let null = Value::Null;
    let p = e.get("Properties").unwrap_or(&null);
    let concurrency = if b(p, "bOverrideConcurrency", false) {
        vec![settings("", p.get("ConcurrencyOverrides").unwrap_or(&null))]
    } else {
        p.get("ConcurrencySet").and_then(Value::as_array).map(|a| a.iter().filter_map(|c| concurrency(rd, &obj(Some(c)))).collect()).unwrap_or_default()
    };
    Some(Routing { class: obj(p.get("SoundClassObject")), concurrency, max_distance: p.get("MaxDistance").and_then(Value::as_f64), submix: obj(p.get("SoundSubmixObject")) })
}

/// every class reachable from the default class's root, and the mixes they reference
#[derive(Debug, Clone, Default)]
pub struct ClassTree {
    pub root: String,
    pub classes: HashMap<String, SoundClass>,
    pub mixes: HashMap<String, SoundMix>,
}

impl ClassTree {
    pub fn load(rd: &Reader) -> ClassTree {
        let mut t = ClassTree::default();
        // the root: follow ParentClass up from the default class
        let mut root = DEFAULT_CLASS.to_string();
        for _ in 0..32 {
            match class(rd, &root) {
                Some(c) if !c.parent.is_empty() => root = c.parent,
                _ => break,
            }
        }
        t.root = root.clone();
        let mut stack = vec![root];
        while let Some(p) = stack.pop() {
            if t.classes.contains_key(&p) {
                continue;
            }
            let Some(c) = class(rd, &p) else { continue };
            stack.extend(c.children.iter().cloned());
            for m in &c.passive {
                if !t.mixes.contains_key(&m.mix) {
                    if let Some(x) = mix(rd, &m.mix) {
                        t.mixes.insert(m.mix.clone(), x);
                    }
                }
            }
            t.classes.insert(p, c);
        }
        t
    }

    /// a class not reached from the root (e.g. one only a cue references) and its mixes
    pub fn add(&mut self, rd: &Reader, path: &str) {
        if path.is_empty() || self.classes.contains_key(path) {
            return;
        }
        if let Some(c) = class(rd, path) {
            for m in &c.passive {
                if let Some(x) = mix(rd, &m.mix) {
                    self.mixes.entry(m.mix.clone()).or_insert(x);
                }
            }
            let parent = c.parent.clone();
            self.classes.insert(path.to_string(), c);
            self.add(rd, &parent);
        }
    }

    /// UMordhauUtilityLibrary::SetSoundMixVolume 0x163e160 for one settings mix: GetSoundMixInfo 0x1628cc0 walks every
    /// loaded USoundClass, and a class whose PassiveSoundMixModifiers (+0xb8, 16-byte entries) name a mix called
    /// `<name>SoundMix` (FName::ToString, Stricmp; the first match per class) gets Properties.Volume (+0x28) =
    /// multiplier x Volume. Names / multipliers from SetSoundMixVolume: 0 "GlobalMaster" (x CMasterVolumeMultiplier,
    /// cvar default 1.0, static init 0x6438f0), 1 "Effects", 2 "Music", 3 "Voice", 4 "Instruments" (x 1). Called by
    /// UMordhauGameUserSettings::ApplyNonResolutionSettings 0x1582870 (startup / settings apply) and
    /// ApplyAudioVolumes 0x15825d0. Returns the classes set.
    pub fn apply_mix_volume(&mut self, name: &str, volume: f64) -> Vec<String> {
        let want = format!("{name}SoundMix").to_ascii_lowercase();
        let mut set = vec![];
        for (k, c) in self.classes.iter_mut() {
            if c.passive.iter().any(|m| m.mix.rsplit('/').next().unwrap_or("").to_ascii_lowercase() == want) {
                c.props.volume = volume;
                set.push(k.clone());
            }
        }
        set.sort();
        set
    }

    fn descendants(&self, c: &str, out: &mut Vec<String>) {
        if let Some(k) = self.classes.get(c) {
            for ch in &k.children {
                if !out.contains(ch) {
                    out.push(ch.clone());
                    self.descendants(ch, out);
                }
            }
        }
    }

    /// effective properties of every class with the active mixes (mix path, InterpValue 0..1)
    pub fn effective(&self, mixes: &[(String, f64)]) -> HashMap<String, ClassProps> {
        let mut out: HashMap<String, ClassProps> = self.classes.iter().map(|(k, c)| (k.clone(), c.props)).collect();
        // RecurseIntoSoundClasses from the root(s): every class whose parent is not loaded starts a tree
        let mut roots: Vec<String> = self.classes.values().filter(|c| c.parent.is_empty() || !self.classes.contains_key(&c.parent)).map(|c| c.path.clone()).collect();
        roots.sort();
        let mut stack = roots;
        let mut seen = vec![];
        while let Some(p) = stack.pop() {
            if seen.contains(&p) {
                continue;
            }
            seen.push(p.clone());
            let Some(c) = self.classes.get(&p) else { continue };
            let pp = out[&p];
            for ch in &c.children {
                if let Some(x) = out.get_mut(ch) {
                    x.volume *= pp.volume;
                    x.pitch *= pp.pitch;
                    x.is_ui |= pp.is_ui;
                    x.is_music |= pp.is_music;
                }
                stack.push(ch.clone());
            }
        }
        // ApplyClassAdjusters per active mix
        for (m, interp) in mixes {
            let Some(mx) = self.mixes.get(m) else { continue };
            for e in &mx.effects {
                let fv = interp * e.volume + 1.0 - interp;
                let fp = interp * e.pitch + 1.0 - interp;
                let mut targets = vec![e.class.clone()];
                if e.apply_to_children {
                    self.descendants(&e.class, &mut targets);
                }
                for t in targets {
                    if let Some(x) = out.get_mut(&t) {
                        x.volume *= fv;
                        x.pitch *= fp;
                    }
                }
            }
        }
        out
    }
}

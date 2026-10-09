//! Sound classes, passive sound mixes and concurrency for the playing sounds (engine-neutral; data from
//! mh_assets::sound_class, rvas there and below).
//!
//! - Class: a sound's SoundClassObject (else the default class); its effective Volume / Pitch (class tree x active
//!   mixes, sound_class::ClassTree::effective) multiply the wave's volume / pitch every frame (FActiveSound parse
//!   params x SoundClassProperties).
//! - Passive mixes: FAudioDevice::UpdatePassiveSoundMixModifiers 0x2f01b90: a playing wave whose volume (x
//!   attenuation, 0x2f01c51) lies in [MinVolumeThreshold, MaxVolumeThreshold] (0x2f01c80 / 0x2f01c8b) keeps its
//!   class's passive mixes active. The mix fades in over FadeInTime and out over FadeOutTime (linear, UNCONFIRMED:
//!   FSoundMixState not read).
//! - Concurrency (FSoundConcurrencyManager::EvaluateConcurrency 0x345c850, not read instruction by instruction; the
//!   rules as their names in EMaxConcurrentResolutionRule state, UNCONFIRMED in detail): one group per
//!   SoundConcurrency asset (bLimitToOwner is false in every Mordhau asset); a new sound past MaxCount: PreventNew ->
//!   not played; StopOldest -> the oldest stops; StopFarthestThen* -> the farthest stops unless the new sound is
//!   farther (then not played), ties -> PreventNew / Oldest. RetriggerTime: not played within it of the group's
//!   newest start. Stopped sounds end at once (VoiceStealReleaseTime fade not ported). VolumeScale is 1 in every
//!   Mordhau asset (no ducking generations).

use mh_assets::sound_class::{ClassProps, ClassTree, Concurrency, Rule};
use std::collections::HashMap;

/// one active sound (a PlayCue start) in a concurrency group
#[derive(Clone, Debug)]
pub struct Member {
    pub inst: u64,
    pub start: f64,
    pub dist_cm: f64,
}

/// the outcome of a concurrency check
#[derive(Clone, Debug, PartialEq)]
pub enum Verdict {
    Play { stop: Vec<u64> },
    Reject(&'static str),
}

/// a group key: the concurrency asset, or "override:<cue>" for ConcurrencyOverrides
pub fn group_key(c: &Concurrency, cue: &str) -> String {
    if c.path.is_empty() {
        format!("override:{cue}")
    } else {
        c.path.clone()
    }
}

/// decide whether a new sound at `dist_cm` may play in a group (members = the group's active sounds)
pub fn evaluate(c: &Concurrency, members: &[Member], now: f64, dist_cm: f64) -> Verdict {
    if c.retrigger_time > 0.0 && members.iter().any(|m| now - m.start < c.retrigger_time) {
        return Verdict::Reject("retrigger");
    }
    if (members.len() as i64) < c.max_count.max(0) {
        return Verdict::Play { stop: vec![] };
    }
    let oldest = || members.iter().min_by(|a, b| a.start.total_cmp(&b.start).then(a.inst.cmp(&b.inst))).map(|m| m.inst);
    match c.rule {
        Rule::PreventNew => Verdict::Reject("prevent_new"),
        Rule::StopFarthestThenPreventNew | Rule::StopFarthestThenOldest => {
            let far = members.iter().max_by(|a, b| a.dist_cm.total_cmp(&b.dist_cm));
            match far {
                Some(f) if dist_cm > f.dist_cm => Verdict::Reject("farthest"),
                Some(f) if dist_cm < f.dist_cm => Verdict::Play { stop: vec![f.inst] },
                _ if c.rule == Rule::StopFarthestThenPreventNew => Verdict::Reject("prevent_new"),
                _ => Verdict::Play { stop: oldest().into_iter().collect() },
            }
        }
        // StopOldest; StopLowestPriority / StopQuietest / *ThenPreventNew are not used by Mordhau's assets
        _ => Verdict::Play { stop: oldest().into_iter().collect() },
    }
}

/// passive mix state
#[derive(Clone, Debug, Default)]
pub struct MixState {
    pub interp: f64,
}

/// class / mix state for the frame
#[derive(Default)]
pub struct Classes {
    pub tree: ClassTree,
    pub mixes: HashMap<String, MixState>,
    pub effective: HashMap<String, ClassProps>,
}

impl Classes {
    pub fn new(tree: ClassTree) -> Classes {
        let effective = tree.effective(&[]);
        Classes { tree, mixes: HashMap::new(), effective }
    }

    /// a class's effective properties (the default class when unknown / empty)
    pub fn props(&self, class: &str) -> ClassProps {
        let k = if class.is_empty() { mh_assets::sound_class::DEFAULT_CLASS } else { class };
        self.effective.get(k).copied().unwrap_or(ClassProps { volume: 1.0, pitch: 1.0, lpf: 20000.0, is_ui: false, is_music: false, reverb: true })
    }

    /// advance the passive mixes: `playing` = (class, wave volume x attenuation) of every playing wave
    pub fn update(&mut self, playing: &[(String, f64)], dt: f64) {
        let mut on: Vec<String> = vec![];
        for (class, v) in playing {
            let k = if class.is_empty() { mh_assets::sound_class::DEFAULT_CLASS } else { class.as_str() };
            if let Some(c) = self.tree.classes.get(k) {
                for p in &c.passive {
                    if *v >= p.min_volume && *v <= p.max_volume && !on.contains(&p.mix) {
                        on.push(p.mix.clone());
                    }
                }
            }
        }
        let mut active = vec![];
        let keys: Vec<String> = self.tree.mixes.keys().cloned().collect();
        for m in keys {
            let mx = &self.tree.mixes[&m];
            let st = self.mixes.entry(m.clone()).or_default();
            if on.contains(&m) {
                st.interp = if mx.fade_in > 0.0 { (st.interp + dt / mx.fade_in).min(1.0) } else { 1.0 };
            } else {
                st.interp = if mx.fade_out > 0.0 { (st.interp - dt / mx.fade_out).max(0.0) } else { 0.0 };
            }
            if st.interp > 0.0 {
                active.push((m.clone(), st.interp));
            }
        }
        active.sort_by(|a, b| a.0.cmp(&b.0));
        self.effective = self.tree.effective(&active);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn conc(max: i64, rule: Rule) -> Concurrency {
        Concurrency {
            path: "c".into(),
            max_count: max,
            limit_to_owner: false,
            rule,
            retrigger_time: 0.0,
            volume_scale: 1.0,
            volume_scale_attack: 0.01,
            volume_scale_can_release: false,
            volume_scale_release: 0.5,
            voice_steal_release: 0.0,
        }
    }

    #[test]
    fn resolution_rules() {
        let m = |inst, start, dist_cm| Member { inst, start, dist_cm };
        let full = vec![m(1, 0.0, 100.0), m(2, 1.0, 500.0)];
        assert_eq!(evaluate(&conc(3, Rule::PreventNew), &full, 2.0, 50.0), Verdict::Play { stop: vec![] });
        assert_eq!(evaluate(&conc(2, Rule::PreventNew), &full, 2.0, 50.0), Verdict::Reject("prevent_new"));
        assert_eq!(evaluate(&conc(2, Rule::StopOldest), &full, 2.0, 50.0), Verdict::Play { stop: vec![1] });
        assert_eq!(evaluate(&conc(2, Rule::StopFarthestThenOldest), &full, 2.0, 50.0), Verdict::Play { stop: vec![2] });
        assert_eq!(evaluate(&conc(2, Rule::StopFarthestThenOldest), &full, 2.0, 900.0), Verdict::Reject("farthest"));
        assert_eq!(evaluate(&conc(2, Rule::StopFarthestThenOldest), &full, 2.0, 500.0), Verdict::Play { stop: vec![1] });
    }
}

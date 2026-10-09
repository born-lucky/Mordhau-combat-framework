//! A minimal animation stand-in for headless runs and tests (NOT the game's animation layer; the ported graph is
//! animgraph.rs, `Sim::enable_anim`): while a fighter's motion is an attack whose class Animation is an AnimSequence
//! (e.g. RawClips/2H/Sword/2H_Sword_RightStrike), the clip is posed at the montage position UAttackMotion::OnTick
//! sets (rust-combat r4, rust-parity r6 finding: the old rate-1 mapping ignored the windup and put every contact at
//! 0.70-0.75 s): SetAnimPosition rva=0x14a0380 with NewPosition = (LastReleaseNormalizedTime + 1) * 0.5 (decomp
//! UAttackMotion.cpp 2836; disasm 0x141633066..0x141633571): Windup -> WindupCurve(normalized windup time) mapped
//! to [0, 0.5], Release -> [0.5, 1] (ReleaseCurve), Recovery from 1.0 at rate 1. UNCONFIRMED / simplified against
//! animgraph.rs attack_position: no AutoBlend windup offset (0), no early-release split, no blends; every other
//! motion shows the reference pose.

use crate::sim::{PoseSource, Sim};
use mh_assets::anim::AnimSequence;
use mh_assets::pak_source::PakSource;
use std::collections::HashMap;
use std::rc::Rc;

pub struct ClipPoser {
    pub src: PakSource,
    cache: HashMap<String, Option<Rc<AnimSequence>>>,
}

impl ClipPoser {
    pub fn new(src: PakSource) -> ClipPoser {
        ClipPoser { src, cache: HashMap::new() }
    }

    fn clip(&mut self, path: &str) -> Option<Rc<AnimSequence>> {
        if !self.cache.contains_key(path) {
            let c = mh_assets::anim::decode(&self.src, path).ok().map(Rc::new);
            self.cache.insert(path.to_string(), c);
        }
        self.cache[path].clone()
    }

    /// Set every fighter's pose for the coming step (call before Sim::step)
    pub fn pose(&mut self, s: &mut Sim) {
        let t_next = s.combat.now + s.dt as f64;
        for fi in 0..s.combat.fighters.len() {
            let clip = s.combat.cur_m(fi).filter(|m| m.is_attack()).map(|m| (m.def.attack().animation.clone(), clip_position(&s.combat.spec, m, t_next)));
            let pose = match clip {
                Some((path, pos)) if !path.is_empty() => self.clip(&path).map(|c| {
                    let t = (pos as f32).clamp(0.0, c.sequence_length);
                    PoseSource::Host(s.geo.skeleton.sample(&c, t))
                }),
                _ => None,
            };
            s.poses[fi] = pose.unwrap_or(PoseSource::Reference);
        }
    }
}

/// The montage position (seconds) UAttackMotion::OnTick would set at `now` (module docs)
pub fn clip_position(spec: &mordhau_core::data::Spec, m: &mordhau_core::combat::Motion, now: f64) -> f64 {
    let Some(a) = m.attack() else { return 0.0 };
    let (st, we, re) = (m.start_time, a.windup_end, a.release_end);
    if now <= we {
        let mut x = mordhau_core::ue::normalized_time(st, we, now);
        if !a.windup_curve.is_empty() {
            x = spec.curve_value(&a.windup_curve, x);
        }
        return 0.5 * x;
    }
    if now <= re {
        let mut r = mordhau_core::ue::normalized_time(we, re, now);
        if !a.release_curve.is_empty() {
            r = spec.curve_value(&a.release_curve, r);
        }
        return (r + 1.0) * 0.5;
    }
    1.0 + (now - re)
}

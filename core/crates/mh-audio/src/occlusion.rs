//! Sound occlusion as the exe does it: a line trace from the listener to the sound on the attenuation's
//! OcclusionTraceChannel; while blocked the sound's occlusion LPF / volume ease to the attenuation's
//! OcclusionLowPassFilterFrequency / OcclusionVolumeAttenuation over OcclusionInterpolationTime.
//!
//! - FActiveSound::UpdateAttenuation 0x2e3f340: only when bEnableOcclusion (+0xb0 bit 5, 0x2e3f91e) and the
//!   parse volume is > 0 (0x2e3f99f); trace from the listener location to the sound's (0x2e3f9d2..0x2e3fa28);
//!   afterwards VolumeMultiplier x= occlusion volume (0x2e3fa2d), OcclusionFilterFrequency = occlusion LPF
//!   (0x2e3fa62).
//! - FActiveSound::CheckOcclusion 0x2e23880: OcclusionCheckInterval (+0x208, 0 from the ctor 0x2e1e43b) since the
//!   last check (+0x20c, -FLT_MAX) -> trace. The first check is a synchronous UWorld::LineTraceTestByChannel
//!   (0x2e239b9) ignoring the owner, channel +0xb9, complex flag +0xb0 bit 6; later ones are async (result next
//!   frame). Interp time: 0 on the first check, else OcclusionInterpolationTime (0x2e23abe). Occluded: the LPF target
//!   only moves down to OcclusionLowPassFilterFrequency (0x2e23ae2), the volume target only down to
//!   OcclusionVolumeAttenuation (0x2e23b02); clear: LPF 20000, volume 1 (0x2e23b0d). Then both
//!   FDynamicParameter::Update(1/30) (0x2e23b3b: a constant 1/30 s per check, not the frame time).
//! - FDynamicParameter ctor (FActiveSound ctor 0x2e1e405: LPF 20000, volume 1), FDynamicParameter::Set 0x2f1fae0,
//!   FDynamicParameter::Update 0x2f23780.
//! - FSoundSource::SetFilterFrequency 0x2efd850: the source cutoff is the minimum of the wave instance's LPF
//!   frequencies (class, occlusion, ambient zone, attenuation).
//!
//! UNCONFIRMED: the trace runs every check synchronously (the exe's async traces land a frame later); the traced
//! world is mh-level's static CollisionWorld (simple collision, no characters / dynamic actors, so the owner never
//! needs ignoring).

use std::sync::Arc;

/// FDynamicParameter: +0 current, +4 start, +8 delta, +0xc current time, +0x10 interp time, +0x18 target
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DynParam {
    pub current: f32,
    start: f32,
    delta: f32,
    time: f32,
    interp: f32,
    pub target: f32,
}

impl DynParam {
    pub fn new(v: f32) -> DynParam {
        DynParam { current: v, start: v, delta: 0.0, time: 0.0, interp: 0.0, target: v }
    }
    /// Set 0x2f1fae0
    pub fn set(&mut self, target: f32, time: f32) {
        if target == self.target && time == self.interp {
            return;
        }
        self.target = target;
        if time > 0.0 {
            self.start = self.current;
            self.delta = target - self.current;
            self.interp = time;
            self.time = 0.0;
        } else {
            self.start = target;
            self.current = target;
            self.delta = 0.0;
            self.interp = 0.0;
        }
    }
    /// Update 0x2f23780 (the fraction is taken before the time advances)
    pub fn update(&mut self, dt: f32) {
        if self.interp <= 0.0 {
            return;
        }
        let t = self.time / self.interp;
        if t < 1.0 {
            self.current = self.start + self.delta * t;
        } else {
            self.current = self.start + self.delta;
            self.interp = 0.0;
        }
        self.time += dt;
    }
}

/// a line trace on a channel: (start UE cm, end UE cm, channel display name e.g. "Camera") -> blocked
pub type Tracer = Arc<dyn Fn([f64; 3], [f64; 3], &str) -> bool + Send + Sync>;

/// the host's occlusion tracer (bevy-runtime inserts it with the map's collision world)
#[derive(bevy::prelude::Resource, Clone)]
pub struct OcclusionTracer(pub Tracer);

/// a tracer over mh-level's collision world: the first body that queries and blocks the channel
pub fn collision_tracer(cw: Arc<mh_level::collision::CollisionWorld>) -> Tracer {
    Arc::new(move |a, b, ch| {
        let cw2 = cw.clone();
        let f = move |body: u32| cw2.bodies.get(body as usize).is_some_and(|x| x.queries() && x.response(ch) == mh_level::collision::Resp::Block);
        cw.trace(a, b, &f).is_some()
    })
}

/// "ECC_Camera" / "ECollisionChannel::ECC_Camera" -> "Camera" (engine channels; game channels pass through)
pub fn channel_name(ecc: &str) -> String {
    let t = ecc.rsplit("::").next().unwrap_or(ecc);
    let t = t.strip_prefix("ECC_").unwrap_or(t);
    t.to_string()
}

/// one sound's occlusion state (FActiveSound +0x180 bit 0, +0x188, +0x1c4, +0x1e0)
#[derive(Clone, Copy, Debug)]
pub struct SoundOcclusion {
    pub checked: bool,
    pub occluded: bool,
    pub lpf: DynParam,
    pub volume: DynParam,
}

impl Default for SoundOcclusion {
    fn default() -> Self {
        SoundOcclusion { checked: false, occluded: false, lpf: DynParam::new(20000.0), volume: DynParam::new(1.0) }
    }
}

impl SoundOcclusion {
    /// UpdateAttenuation's occlusion step: returns (volume multiplier, occlusion LPF)
    pub fn update(&mut self, att: Option<&mh_assets::sound_cue::Attenuation>, audible: bool, trace: impl FnOnce(&str) -> bool) -> (f32, f32) {
        let Some(a) = att.filter(|a| a.enable_occlusion) else { return (1.0, 20000.0) };
        if !audible {
            return (self.volume.current, self.lpf.current);
        }
        self.occluded = trace(&channel_name(&a.occlusion_trace_channel));
        let time = if self.checked { a.occlusion_interpolation_time as f32 } else { 0.0 };
        self.checked = true;
        if self.occluded {
            let f = a.occlusion_lpf as f32;
            if f < self.lpf.target {
                self.lpf.set(f, time);
            }
            let v = a.occlusion_volume as f32;
            if v < self.volume.target {
                self.volume.set(v, time);
            }
        } else {
            self.lpf.set(20000.0, time);
            self.volume.set(1.0, time);
        }
        self.lpf.update(1.0 / 30.0);
        self.volume.update(1.0 / 30.0);
        (self.volume.current, self.lpf.current)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// BaseAttenuation-like occlusion (volume 0.4 over 0.25 s, here with a 2000 Hz LPF): the first check applies
    /// at once, later ones ease in 1/30 s steps
    #[test]
    fn occlusion_eases_after_first_check() {
        let mut a = mh_assets::sound_cue::attenuation_settings(&serde_json::json!({"bEnableOcclusion": true, "OcclusionVolumeAttenuation": 0.4, "OcclusionInterpolationTime": 0.25, "OcclusionTraceChannel": "ECC_Camera"}));
        a.occlusion_lpf = 2000.0;
        let mut o = SoundOcclusion::default();
        let (v, f) = o.update(Some(&a), true, |ch| {
            assert_eq!(ch, "Camera");
            true
        });
        assert_eq!((v, f), (0.4, 2000.0));
        let mut o = SoundOcclusion::default();
        o.update(Some(&a), true, |_| false);
        let (v1, _) = o.update(Some(&a), true, |_| true);
        assert_eq!(v1, 1.0);
        let (v2, _) = o.update(Some(&a), true, |_| true);
        assert!((v2 - (1.0 - 0.6 * (1.0 / 30.0) / 0.25)).abs() < 1e-5, "{v2}");
        for _ in 0..10 {
            o.update(Some(&a), true, |_| true);
        }
        // start + delta (Update 0x2f237c5), not the target: f32 rounding as in the exe
        assert!((o.volume.current - 0.4).abs() < 1e-6);
    }
}

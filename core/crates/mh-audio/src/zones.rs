//! Ambient zones: AudioVolume AmbientZoneSettings (FInteriorSettings) applied to sounds as a volume multiplier
//! and a low-pass cutoff, as the exe's audio device does it.
//!
//! - FInteriorSettings ctor 0x2f06ea0 (layout and defaults): bIsWorldSettings +0 (false), ExteriorVolume +4 (1),
//!   ExteriorTime +8 (0.5), ExteriorLPF +0xc (20000), ExteriorLPFTime +0x10 (0.5), InteriorVolume +0x14 (1),
//!   InteriorTime +0x18 (0.5), InteriorLPF +0x1c (20000), InteriorLPFTime +0x20 (0.5).
//! - FAudioDevice::GetAudioVolumeSettings 0x2ef39e0: the first audio volume (of the world) whose body contains the
//!   point (GetSquaredDistanceToBody == 0, 0x2ef3be5) gives its ID / reverb / interior settings; else ID 0 and the
//!   world's defaults (AWorldSettings DefaultAmbientZoneSettings +0x2b8, handed over in PostRegisterAllComponents
//!   0x357a7f3 -> SetDefaultAudioSettings 0x2efd440). No Mordhau map stores DefaultAmbientZoneSettings, so the
//!   world default is the ctor default.
//! - Listener: FAudioDevice::UpdateAudioVolumeEffects 0x2f00d80 (listener interior settings at +0x4c, the volume
//!   ID at +0x70, InteriorStartTime +0x80, end times +0x88..+0xa0, interps +0xa8..+0xb4).
//! - Each sound: FActiveSound::HandleInteriorVolumes 0x2e2fde0 (+0x660 the sound's settings, +0x6a8 its volume
//!   ID, +0x6b0 LastUpdateTime, +0x6b8 / +0x6bc source volume / LPF, +0x6c0 / +0x6c4 current volume / LPF).
//!
//! UNCONFIRMED: containment uses the boxes of the brush's convex elements (ambient::volume_at), not the exact
//! convex body; whether the world default has bIsWorldSettings set (no writer of +0x2b8 found; with every map on
//! ctor defaults both branches give the same targets, only the interp clock differs).

use serde_json::Value;

/// MAX_FILTER_FREQUENCY (the same-zone LPF target, 0x2e302ae)
pub const MAX_LPF: f32 = 20000.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InteriorSettings {
    pub is_world: bool,
    pub exterior_volume: f32,
    pub exterior_time: f32,
    pub exterior_lpf: f32,
    pub exterior_lpf_time: f32,
    pub interior_volume: f32,
    pub interior_time: f32,
    pub interior_lpf: f32,
    pub interior_lpf_time: f32,
}

impl Default for InteriorSettings {
    /// FInteriorSettings ctor 0x2f06ea0
    fn default() -> Self {
        InteriorSettings {
            is_world: false,
            exterior_volume: 1.0,
            exterior_time: 0.5,
            exterior_lpf: MAX_LPF,
            exterior_lpf_time: 0.5,
            interior_volume: 1.0,
            interior_time: 0.5,
            interior_lpf: MAX_LPF,
            interior_lpf_time: 0.5,
        }
    }
}

impl InteriorSettings {
    /// an AmbientZoneSettings struct as a volume stores it (fields it omits keep the ctor defaults)
    pub fn from_json(v: Option<&Value>) -> InteriorSettings {
        let d = InteriorSettings::default();
        let Some(v) = v else { return d };
        let f = |k: &str, x: f32| v.get(k).and_then(Value::as_f64).map_or(x, |y| y as f32);
        InteriorSettings {
            is_world: v.get("bIsWorldSettings").and_then(Value::as_bool).unwrap_or(false),
            exterior_volume: f("ExteriorVolume", d.exterior_volume),
            exterior_time: f("ExteriorTime", d.exterior_time),
            exterior_lpf: f("ExteriorLPF", d.exterior_lpf),
            exterior_lpf_time: f("ExteriorLPFTime", d.exterior_lpf_time),
            interior_volume: f("InteriorVolume", d.interior_volume),
            interior_time: f("InteriorTime", d.interior_time),
            interior_lpf: f("InteriorLPF", d.interior_lpf),
            interior_lpf_time: f("InteriorLPFTime", d.interior_lpf_time),
        }
    }
}

/// the zone a point is in: volume ID (0 = the world) and its settings
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zone {
    pub id: u32,
    pub settings: InteriorSettings,
}

/// GetAudioVolumeSettings 0x2ef39e0 over the map's volumes (ID = index + 1)
pub fn zone_at(vols: &[crate::ambient::AudioVol], p: [f64; 3]) -> Zone {
    match crate::ambient::volume_at(vols, p) {
        Some(v) => Zone {
            id: vols.iter().position(|w| std::ptr::eq(w, v)).map_or(0, |i| i as u32 + 1),
            settings: InteriorSettings::from_json(v.ambient_zone.as_ref()),
        },
        None => Zone { id: 0, settings: InteriorSettings::default() },
    }
}

/// the listener's ambient zone state (FListener +0x4c..+0xb4)
#[derive(Clone, Copy, Debug)]
pub struct ListenerZone {
    pub id: u32,
    pub settings: InteriorSettings,
    pub start: f64,
    pub interior_end: f64,
    pub exterior_end: f64,
    pub interior_lpf_end: f64,
    pub exterior_lpf_end: f64,
    pub interior_volume_interp: f32,
    pub interior_lpf_interp: f32,
    pub exterior_volume_interp: f32,
    pub exterior_lpf_interp: f32,
}

impl Default for ListenerZone {
    /// FListener ctor: ID -1 (0x2f00e80 initializes the queried ID to -1), interps 0 until the first update
    fn default() -> Self {
        ListenerZone {
            id: u32::MAX,
            settings: InteriorSettings::default(),
            start: 0.0,
            interior_end: 0.0,
            exterior_end: 0.0,
            interior_lpf_end: 0.0,
            exterior_lpf_end: 0.0,
            interior_volume_interp: 0.0,
            interior_lpf_interp: 0.0,
            exterior_volume_interp: 0.0,
            exterior_lpf_interp: 0.0,
        }
    }
}

/// 0x2f00ff4..0x2f01114: 0 before start, 1 at/after end, else the clamped fraction
fn interp(now: f64, start: f64, end: f64) -> f32 {
    if now < start {
        0.0
    } else if now >= end {
        1.0
    } else {
        (((now - start) / (end - start)) as f32).clamp(0.0, 1.0)
    }
}

impl ListenerZone {
    /// UpdateAudioVolumeEffects 0x2f00d80: on a new volume ID or different settings (0x2f00f09 / 0x2f00f16) the
    /// interpolation restarts now; each end time uses the new zone's time, or the old zone's when the new one is
    /// the world's (0x2f00f4b..0x2f00fc3). Then the four interps (0x2f00fe4..0x2f01114).
    pub fn update(&mut self, z: Zone, now: f64) {
        if z.id != self.id || z.settings != self.settings {
            let pick = |new: f32, old: f32| if z.settings.is_world { old } else { new } as f64;
            self.start = now;
            self.interior_end = now + pick(z.settings.interior_time, self.settings.interior_time);
            self.exterior_end = now + pick(z.settings.exterior_time, self.settings.exterior_time);
            self.interior_lpf_end = now + pick(z.settings.interior_lpf_time, self.settings.interior_lpf_time);
            self.exterior_lpf_end = now + pick(z.settings.exterior_lpf_time, self.settings.exterior_lpf_time);
            self.id = z.id;
            self.settings = z.settings;
        }
        self.interior_volume_interp = interp(now, self.start, self.interior_end);
        self.exterior_volume_interp = interp(now, self.start, self.exterior_end);
        self.interior_lpf_interp = interp(now, self.start, self.interior_lpf_end);
        self.exterior_lpf_interp = interp(now, self.start, self.exterior_lpf_end);
    }
}

/// one sound's interior state (FActiveSound +0x6b0..+0x6c4)
#[derive(Clone, Copy, Debug)]
pub struct SoundZone {
    pub last_update: f64,
    pub source_volume: f32,
    pub source_lpf: f32,
    pub current_volume: f32,
    pub current_lpf: f32,
}

impl Default for SoundZone {
    /// FActiveSound ctor: UNCONFIRMED initial values (taken as no effect: 1 and MAX_LPF)
    fn default() -> Self {
        SoundZone { last_update: f64::MIN, source_volume: 1.0, source_lpf: MAX_LPF, current_volume: 1.0, current_lpf: MAX_LPF }
    }
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

impl SoundZone {
    /// HandleInteriorVolumes 0x2e2fde0: returns (InteriorVolumeMultiplier, AmbientZoneFilterFrequency).
    /// `spatialized` = the bit tested at 0x2e3001a (a non-spatialized sound takes the same-zone branch).
    pub fn apply(&mut self, lis: &ListenerZone, sound: Zone, spatialized: bool, now: f64) -> (f32, f32) {
        // 0x2e2ffd1: the listener's zone changed after this sound's last update: the current values become the
        // interpolation sources
        if lis.start > self.last_update {
            self.source_volume = self.current_volume;
            self.source_lpf = self.current_lpf;
            self.last_update = now;
        }
        let (v, f) = if lis.id == sound.id || !spatialized {
            // 0x2e30278: same zone
            (lerp(self.source_volume, 1.0, lis.interior_volume_interp), lerp(self.source_lpf, MAX_LPF, lis.interior_lpf_interp))
        } else if sound.settings.is_world {
            // 0x2e30034: the sound is outside: the listener zone's exterior values
            (
                lerp(self.source_volume, lis.settings.exterior_volume, lis.exterior_volume_interp),
                lerp(self.source_lpf, lis.settings.exterior_lpf, lis.exterior_lpf_interp),
            )
        } else {
            // 0x2e30138: the sound is inside another zone: its interior x the listener's exterior; LPF the lower
            let v = lerp(self.source_volume, sound.settings.interior_volume, lis.interior_volume_interp)
                * lerp(self.source_volume, lis.settings.exterior_volume, lis.exterior_volume_interp);
            let a = lerp(self.source_lpf, sound.settings.interior_lpf, lis.interior_lpf_interp);
            let b = lerp(self.source_lpf, lis.settings.exterior_lpf, lis.exterior_lpf_interp);
            (v, if a < b { a } else { b })
        };
        self.current_volume = v;
        self.current_lpf = f;
        (v, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the listener walks into a zone with ExteriorLPF 3000 (Grad's BarnVolume): a world sound outside ramps to
    /// 3000 Hz over ExteriorLPFTime, a sound in the same zone stays open
    #[test]
    fn barn_exterior_lpf() {
        let barn = InteriorSettings { exterior_lpf: 3000.0, exterior_lpf_time: 0.2, ..Default::default() };
        let mut l = ListenerZone::default();
        l.update(Zone { id: 0, settings: InteriorSettings::default() }, 0.0);
        let mut outside = SoundZone::default();
        let mut inside = SoundZone::default();
        let world = Zone { id: 0, settings: InteriorSettings { is_world: true, ..Default::default() } };
        let z = Zone { id: 1, settings: barn };
        outside.apply(&l, world, true, 0.0);
        l.update(z, 1.0);
        let (_, f0) = outside.apply(&l, world, true, 1.0);
        assert_eq!(f0, MAX_LPF);
        l.update(z, 1.1);
        let (_, f1) = outside.apply(&l, world, true, 1.1);
        assert!((f1 - (MAX_LPF + 3000.0) / 2.0).abs() < 1.0, "{f1}");
        l.update(z, 1.3);
        let (v, f2) = outside.apply(&l, world, true, 1.3);
        assert_eq!((v, f2), (1.0, 3000.0));
        assert_eq!(inside.apply(&l, z, true, 1.3), (1.0, MAX_LPF));
    }
}

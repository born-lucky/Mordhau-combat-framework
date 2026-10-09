//! The audio mixer's channel maps for stereo output (Mordhau-Win64-Shipping.exe, rvas from extract/split/symbols.tsv,
//! read with scripts/ue_dis.py). Engine-neutral: directions are plain vectors in any one frame.
//!
//! - Azimuth: FAudioDevice::GetAzimuth 0x2ef3db0. f = clamp(dot(listener forward, listener-to-sound dir), -1, 1),
//!   r = dot(listener right, dir); (f, r) normalised when its squared length > 1e-8; angle = pi/2 when |f| <= 1e-8,
//!   else atan(r / f); absolute azimuth = |degrees(angle)|, then f > 0: r < 0 -> 360 - a; f < 0: r >= 0 -> 180 - a
//!   (r == 0 keeps a), r < 0 -> 180 + a. 0 = ahead, 90 = right, clockwise.
//!   Corrective boundary exception: native f == 0 leaves a=90 for BOTH sides
//!   (0x2ef3fce jae 0x2ef3ff5). Our axis-aligned camera hits this often; preserve
//!   the side at that boundary so an exactly-left emitter pans left, continuously
//!   with the neighboring quadrants. Other native branches remain unchanged.
//! - Speakers: FMixerDevice::InitializeChannelAzimuthMap 0x2cc4cf0 for 2 output channels: FrontLeft 270,
//!   FrontRight 90 (the ini AudioChannelAzimuthMap is skipped for stereo, 0x2cc4e81).
//! - Mono, spatialized (FMixerSource::ComputeMonoChannelMap 0x2cb6760 -> FMixerDevice::Get3DChannelMap 0x2cbc7a0):
//!   the first speaker whose azimuth >= the sound's is `next`, the one before (wrapping) `prev`; fraction = (az -
//!   prev) / (next - prev) with 360 added to next when next < prev and to az when az < prev. PanningMethod =
//!   UAudioSettings +0x128 (copied by FMixerDevice::InitializeHardware 0x2cc527c), not set by the ctor 0x2f07050 nor
//!   by any ini -> 0 = Linear: next = fraction, prev = 1 - fraction (0x2cbcac6). Omni blend: r = NormalizedOmniRadius^2;
//!   r > 1 -> omni = 1 - 1 / r, each speaker gain = lerp(gain, 1 / 2, omni) (0x2cbcadb..0x2cbcc03).
//!   NormalizedOmniRadius = clamp(OmniRadius / Distance, 0, 1e6) when OmniRadius > 0 (1e6 at distance 0), else 0
//!   (FSoundSource::GetSpatializationParams 0x2ef66f3).
//! - Mono, not spatialized: Get2DChannelMapInternal 0x2cbc370 with MonoChannelUpmixMethod = UAudioSettings +0x129
//!   (InitializeHardware 0x2cc526f; default 0 = Linear) -> 0.5 / 0.5 (0x2cbc518).
//! - Stereo, spatialized (ComputeStereoChannelMap 0x2cb88e0): two Get3DChannelMap calls, left at az - h and right at
//!   az + h (wrapped to [0, 360)), h = degrees(atan(0.5 x StereoSpread / Distance)) when Distance > 0.0001, else left
//!   270 / right 90 (0x2cb897e).
//! - Stereo, not spatialized: L -> L, R -> R (UNCONFIRMED: that branch's Get2DChannelMap table not read).
//! - The map is recomputed only when the azimuth moved by more than 0.01 degrees (0x2cb6851).
//! Spatialization needs attenuation settings with bSpatialize (FWaveInstance::GetUseSpatialization 0x2ef6970);
//! NonSpatializedRadius blending is not ported (UNCONFIRMED).

/// FAudioDevice::GetAzimuth 0x2ef3db0: the absolute azimuth (degrees, 0 ahead, 90 right)
pub fn absolute_azimuth(fwd: [f32; 3], right: [f32; 3], dir: [f32; 3]) -> f32 {
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let mut f = dot(fwd, dir).clamp(-1.0, 1.0);
    let mut r = dot(right, dir);
    let l2 = f * f + r * r;
    if l2 > 1e-8 {
        let inv = 1.0 / l2.sqrt();
        f *= inv;
        r *= inv;
    }
    let ang = if f.abs() <= 1e-8 { std::f32::consts::FRAC_PI_2 } else { (r / f).atan() };
    let a = ang.to_degrees().abs();
    if f > 0.0 {
        if r >= 0.0 {
            a
        } else {
            360.0 - a
        }
    } else if f == 0.0 {
        if r < 0.0 { 360.0 - a } else { a }
    } else if r < 0.0 {
        a + 180.0
    } else if r > 0.0 {
        180.0 - a
    } else {
        a
    }
}

/// Get3DChannelMap 0x2cbc7a0 for stereo output, Linear panning: [left gain, right gain]
pub fn map_3d(az: f32, normalized_omni: f32) -> [f32; 2] {
    // speakers sorted by azimuth: FrontRight 90 (output 1), FrontLeft 270 (output 0)
    const SPK: [(usize, f32); 2] = [(1, 90.0), (0, 270.0)];
    let i = SPK.iter().position(|s| az <= s.1);
    let (next, prev) = match i {
        Some(i) => (SPK[i], SPK[(i + SPK.len() - 1) % SPK.len()]),
        None => (SPK[0], SPK[SPK.len() - 1]),
    };
    let mut na = next.1;
    let mut a = az;
    if na < prev.1 {
        na += 360.0;
    }
    if a < prev.1 {
        a += 360.0;
    }
    let frac = (a - prev.1) / (na - prev.1);
    let (gn, gp) = (frac, 1.0 - frac);
    let r = normalized_omni * normalized_omni;
    let omni = if r > 1.0 { 1.0 - 1.0 / r } else { 0.0 };
    let per = 0.5;
    let mut out = [0f32; 2];
    out[prev.0] = if omni == 0.0 { gp } else { (per - gp) * omni + gp };
    out[next.0] = if omni == 0.0 { gn } else { (per - gn) * omni + gn };
    out
}

/// NormalizedOmniRadius (GetSpatializationParams 0x2ef66f3)
pub fn normalized_omni(omni_radius_cm: f32, dist_cm: f32) -> f32 {
    if omni_radius_cm <= 0.0 {
        0.0
    } else if dist_cm > 0.0 {
        (omni_radius_cm / dist_cm).clamp(0.0, 1e6)
    } else {
        1e6
    }
}

/// the 2x2 channel map (source channel -> [L, R]) of a source
pub fn channel_map(channels: u16, spatialize: bool, az: f32, omni: f32, stereo_spread_cm: f32, dist_cm: f32) -> [[f32; 2]; 2] {
    match (channels, spatialize) {
        (1, true) => [map_3d(az, omni), [0.0, 0.0]],
        (1, false) => [[0.5, 0.5], [0.0, 0.0]],
        (_, true) => {
            let (l, r) = if dist_cm > 0.0001 {
                let h = (0.5 * stereo_spread_cm / dist_cm).atan().to_degrees();
                let mut l = az - h;
                if l < 0.0 {
                    l += 360.0;
                }
                let mut r = az + h;
                if r > 360.0 {
                    r -= 360.0;
                }
                (l, r)
            } else {
                (270.0, 90.0)
            };
            [map_3d(l, omni), map_3d(r, omni)]
        }
        _ => [[1.0, 0.0], [0.0, 1.0]],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn azimuth_quadrants() {
        let (f, r) = ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        assert_eq!(absolute_azimuth(f, r, [1.0, 0.0, 0.0]), 0.0);
        assert!((absolute_azimuth(f, r, [0.0, 1.0, 0.0]) - 90.0).abs() < 1e-4);
        // exactly behind (r == 0) keeps |atan(0)| = 0 (0x2ef3fe3 jbe): the exe's quirk, same pan as 180
        assert_eq!(absolute_azimuth(f, r, [-1.0, 0.0, 0.0]), 0.0);
        assert!((absolute_azimuth(f, r, [0.0, -1.0, 0.0]) - 270.0).abs() < 1e-4);
        assert!((absolute_azimuth(f, r, [1.0, -1.0, 0.0]) - 315.0).abs() < 1e-3);
        assert!((absolute_azimuth(f, r, [-1.0, -1.0, 0.0]) - 225.0).abs() < 1e-3);
        assert!((absolute_azimuth(f, r, [-1.0, 1.0, 0.0]) - 135.0).abs() < 1e-3);
    }

    #[test]
    fn exact_left_boundary_preserves_side_and_camera_rotation() {
        let (f, r) = ([0.0, 0.0, -1.0], [1.0, 0.0, 0.0]);
        for z in [-1e-6, 0.0, 1e-6] {
            let az = absolute_azimuth(f, r, [-1.0, 0.0, z]);
            assert!((az - 270.0).abs() < 0.001, "left boundary discontinuity: {az}");
            let gains = map_3d(az, 0.0);
            assert!(gains[0] > 0.999 && gains[1] < 0.001);
        }
        let az = absolute_azimuth([0.0, 0.0, 1.0], [-1.0, 0.0, 0.0], [-1.0, 0.0, 0.0]);
        assert_eq!(map_3d(az, 0.0), [0.0, 1.0]);
    }

    /// linear panning: ahead = 0.5 / 0.5, right = 0 / 1, left = 1 / 0, behind = 0.5 / 0.5
    #[test]
    fn linear_pan() {
        assert_eq!(map_3d(0.0, 0.0), [0.5, 0.5]);
        assert_eq!(map_3d(90.0, 0.0), [0.0, 1.0]);
        assert_eq!(map_3d(270.0, 0.0), [1.0, 0.0]);
        assert_eq!(map_3d(180.0, 0.0), [0.5, 0.5]);
        assert_eq!(map_3d(45.0, 0.0), [0.25, 0.75]);
        // inside the omni radius both sides move toward 1/2
        let m = map_3d(90.0, 2.0);
        assert!((m[0] - 0.375).abs() < 1e-6 && (m[1] - 0.625).abs() < 1e-6);
    }
}

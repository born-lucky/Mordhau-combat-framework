//! A cooked UE 4.26 AnimSequence read from the paks: its compressed bone tracks decoded to keys (UE units), and the
//! engine's sampler over them. Port of godot/components/ue/pak/ue_anim_sequence.gd (decode) and
//! godot/game/anim/additive_clip.gd (sampling, read off the shipped exe).
//!
//! Every shipped clip the port exports (147 under Animations/) is cooked with DefaultAnimBoneCompressionSettings into
//! AKF_PerTrackCompression (extract/json CompressedDataStructure.KeyEncodingFormat), so that is the codec decoded
//! here; any other key encoding is reported, not guessed.
//!
//! Layout (CUE4Parse at the pinned commit, tools/CUE4Parse-src/CUE4Parse/UE4/Assets/Exports/Animation/...):
//!   UAnimSequence.Deserialize (UAnimSequence.cs:47-130): tagged properties, UObject Guid, FGuid SkeletonGuid
//!     (UAnimationAsset.cs:20-23), FStripDataFlags, bool bSerializeCompressedData, then SerializeCompressedData3
//!     (UAnimSequence.cs:310-336): int32 CompressedRawDataSize, TArray<int32> CompressedTrackToSkeletonMapTable,
//!     TArray<FSmartName = FName> CompressedCurveNames, the serialized byte stream (int32 NumBytes, bool
//!     bUseBulkDataForLoad, the bytes or an FByteBulkData), FString BoneCodecDDCHandle, FString CurveCodecPath, int32 +
//!     curve bytes, then FUECompressedAnimData::SerializeCompressedData (base first for 4.25+,
//!     AnimCompressionTypes.cs:78-140): int32 CompressedNumberOfFrames, uint8 KeyEncodingFormat, Translation /
//!     Rotation / Scale formats, int32 CompressedByteStream size, TrackOffsets count, ScaleOffsets count, StripSize.
//!     The byte stream holds int32 TrackOffsets[] (translation, rotation per track), int32 ScaleOffsets[], then the
//!     keys (InitViewsFromBuffer, AnimCompressionTypes.cs:100-106).
//!   Per-track keys (CUE4Parse-Conversion/Legacy/AnimConverter.cs:332-540, AnimationCompressionUtils.cs; the same
//!     decoders as scripts/anim_additive.py): uint32 header = format << 28 | component mask << 24 | key count; for
//!     IntervalFixed32NoW the per-component (min, range) floats; the keys; align 4; with mask bit 3 the key frame
//!     numbers (uint8 if NumFrames < 256, else uint16), align 4. Offset -1 = no track: rotation identity, translation
//!     zero, scale one (AnimConverter.cs:508-537).

use crate::buf::{le_f32, le_i32, le_u16, le_u32, Buf};
use crate::bulk;
use crate::props::{self, Props};
use crate::{err, skip_object_guid, PackageSource, Result};

/// AnimationKeyFormat (AnimationKeyFormat.cs)
pub const AKF_PER_TRACK_COMPRESSION: u8 = 2;
// AnimationCompressionFormat (AnimationCompressionFormat.cs)
pub const ACF_NONE: u32 = 0;
pub const ACF_FLOAT96_NO_W: u32 = 1;
pub const ACF_FIXED48_NO_W: u32 = 2;
pub const ACF_INTERVAL_FIXED32_NO_W: u32 = 3;
pub const ACF_FIXED32_NO_W: u32 = 4;
pub const ACF_FLOAT32_NO_W: u32 = 5;
pub const ACF_IDENTITY: u32 = 6;

/// Keys of one channel. `frames` empty = keys spread uniformly over the clip; else each key's frame number
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Channel<T> {
    pub keys: Vec<T>,
    pub frames: Vec<f32>,
}

pub type VecTrack = Channel<[f32; 3]>;
/// quaternions (x, y, z, w), UE space
pub type QuatTrack = Channel<[f32; 4]>;

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    /// USkeleton reference-skeleton bone index (CompressedTrackToSkeletonMapTable)
    pub bone: i32,
    pub pos: Option<VecTrack>,
    pub rot: Option<QuatTrack>,
    pub scale: Option<VecTrack>,
}

#[derive(Debug, Clone)]
pub struct AnimSequence {
    pub package: String,
    pub num_frames: i32,
    pub sequence_length: f32,
    pub rate_scale: f32,
    /// as stored, e.g. "AAT_None" / "AAT_LocalSpaceBase" (enum prefix dropped)
    pub additive_anim_type: String,
    /// Interpolation "Step" (EAnimInterpolationType::Step): the sampler holds key 0 of each pair
    pub step: bool,
    pub codec: String,
    pub properties: Props,
    pub tracks: Vec<Track>,
    /// rust-combat r6 (marked addition): the float curves (CompressedCurveNames + the curve bytes of
    /// UAnimCurveCompressionCodec_CompressedRichCurve, decoded as CUE4Parse AnimCurveCompressionCodec_CompressedRichCurve.cs
    /// / RichCurve.cs adapters do); empty when the clip has none or another codec
    pub curves: Vec<FloatCurve>,
}

/// one decoded FRichCurve key (ERichCurveInterpMode: 0 linear, 1 constant, 2 cubic)
#[derive(Debug, Clone, PartialEq)]
pub struct CurveKey {
    pub interp: u8,
    pub time: f32,
    pub value: f32,
    pub arrive_tangent: f32,
    pub leave_tangent: f32,
}

/// a named float curve (FFloatCurve): keys, pre / post extrapolation (ERichCurveExtrapolation: 0 cycle, 1 cycle with
/// offset, 2 oscillate, 3 linear, 4 constant, 5 none), the default value (used without keys)
#[derive(Debug, Clone, PartialEq)]
pub struct FloatCurve {
    pub name: String,
    pub keys: Vec<CurveKey>,
    pub pre: u8,
    pub post: u8,
    pub default_value: f32,
}

/// the curve bytes of UAnimCurveCompressionCodec_CompressedRichCurve: per curve an FCurveDesc (CompressionFormat u8,
/// KeyTimeCompressionFormat u8, PreInfinityExtrap u8, PostInfinityExtrap u8, int32 NumKeys / ConstantValue,
/// int32 KeyDataOffset) then the key blobs. ERichCurveCompressionFormat: 0 Empty, 1 Constant, 2 Linear, 3 Cubic,
/// 4 Mixed, 5 Weighted; key times: 0 uint16 quantized (+ float MinTime, DeltaTime), 1 float32
pub fn decode_curves(names: &[String], data: &[u8]) -> Vec<FloatCurve> {
    let rd_f = |o: usize| if o + 4 <= data.len() { le_f32(data, o) } else { 0.0 };
    let rd_u16 = |o: usize| if o + 2 <= data.len() { le_u16(data, o) } else { 0 };
    let align = |x: usize, a: usize| (x + a - 1) / a * a;
    let mut out = Vec::new();
    for (ci, name) in names.iter().enumerate() {
        let d = ci * 12;
        if d + 12 > data.len() {
            break;
        }
        let (fmt, ktf, pre, post) = (data[d], data[d + 1], data[d + 2], data[d + 3]);
        let n_or_c = le_i32(data, d + 4);
        let base = le_i32(data, d + 8).max(0) as usize;
        let mut c = FloatCurve { name: name.clone(), keys: Vec::new(), pre, post, default_value: f32::MAX };
        match fmt {
            0 => c.default_value = f32::from_bits(n_or_c as u32),
            1 => c.keys.push(CurveKey { interp: 1, time: 0.0, value: f32::from_bits(n_or_c as u32), arrive_tangent: 0.0, leave_tangent: 0.0 }),
            2..=5 => {
                let n = n_or_c.max(0) as usize;
                // interp modes (Mixed: 1 byte per key, Weighted: 2) before the key times
                let modes = match fmt {
                    4 => n,
                    5 => 2 * n,
                    _ => 0,
                };
                let (times_off, key_data, time): (usize, usize, Box<dyn Fn(usize) -> f32>) = if ktf == 0 {
                    let to = base + align(modes, 2);
                    let range = base + align(align(modes, 2) + 2 * n, 4);
                    let (mn, dt) = (rd_f(range), rd_f(range + 4));
                    (to, range + 8, Box::new(move |k| rd_u16(to + 2 * k) as f32 * (1.0 / 65535.0) * dt + mn))
                } else {
                    let to = base + align(modes, 4);
                    (to, to + 4 * n, Box::new(move |k| rd_f(to + 4 * k)))
                };
                let _ = times_off;
                for k in 0..n {
                    // ERichCurveCompressionFormat of the key -> ERichCurveInterpMode (Linear 0, Constant 1, Cubic 2)
                    let kf = match fmt {
                        4 | 5 => data.get(base + if fmt == 5 { 2 * k } else { k }).copied().unwrap_or(2),
                        f => f,
                    };
                    let interp = match kf {
                        1 => 1,
                        3 => 2,
                        _ => 0,
                    };
                    let stride = match fmt {
                        2 => 1,
                        5 => 5,
                        _ => 3,
                    };
                    let h = key_data + 4 * stride * k;
                    let (at, lt) = if stride >= 3 { (rd_f(h + 4), rd_f(h + 8)) } else { (0.0, 0.0) };
                    c.keys.push(CurveKey { interp, time: time(k), value: rd_f(h), arrive_tangent: at, leave_tangent: lt });
                }
            }
            _ => {}
        }
        out.push(c);
    }
    out
}

fn enum_tail(s: &str) -> &str {
    s.rsplit("::").next().unwrap_or(s)
}

/// Decode the package's AnimSequence export
pub fn decode(src: &dyn PackageSource, pkg_path: &str) -> Result<AnimSequence> {
    let pkg = src.package(pkg_path)?;
    let e = match pkg.find_export("AnimSequence", None) {
        Some(e) => e,
        None => return err(format!("{}: no AnimSequence export", pkg_path)),
    };
    let (start, end) = pkg.export_range(e)?;
    let mut r = Buf::at(&pkg.data, start);
    let properties = props::read_tagged(&pkg, &mut r, end);
    skip_object_guid(&mut r, e);
    r.skip(16); // UAnimationAsset SkeletonGuid (UAnimationAsset.cs:20-23)
    r.skip(2); // FStripDataFlags
    if r.s32() == 0 {
        return err(format!("{}: no compressed data", pkg_path)); // bSerializeCompressedData
    }
    r.s32(); // CompressedRawDataSize
    let nt = r.s32();
    if nt < 0 || nt as usize > r.left() / 4 {
        return err(format!("{}: track table of {}", pkg_path, nt));
    }
    let table: Vec<i32> = (0..nt).map(|_| r.s32()).collect();
    let nc = r.s32();
    // CompressedCurveNames (FSmartName serialized as an FName here)
    let curve_names: Vec<String> = if nc > 0 && (nc as usize) <= r.left() / 8 { (0..nc).map(|_| pkg.fname(&mut r)).collect() } else {
        r.skip_n(nc, 8);
        Vec::new()
    };
    let nbytes = r.s32();
    if nbytes < 0 {
        return err(format!("{}: byte stream of {}", pkg_path, nbytes));
    }
    let stream: Vec<u8> = if r.s32() != 0 {
        // bUseBulkDataForLoad
        let h = bulk::header(&pkg, &mut r);
        let mut b = bulk::bytes(src, &pkg, &h)?;
        b.truncate(nbytes as usize);
        b
    } else {
        r.bytes(nbytes as usize).to_vec()
    };
    let codec = r.fstring(); // BoneCodecDDCHandle
    let curve_codec = r.fstring(); // CurveCodecPath
    let cb = r.s32();
    let curve_bytes: Vec<u8> = if cb > 0 && (cb as usize) <= r.left() { r.bytes(cb as usize).to_vec() } else {
        r.skip_n(cb, 1);
        Vec::new()
    };
    // the codec object: DefaultAnimCurveCompressionSettings' CurveCompressionCodec is a
    // UAnimCurveCompressionCodec_CompressedRichCurve (the engine default; CUE4Parse resolves it the same way)
    let curves = if curve_codec.contains("CompressedRichCurve") || curve_codec.contains("DefaultAnimCurveCompressionSettings") { decode_curves(&curve_names, &curve_bytes) } else { Vec::new() };
    let frames = r.s32();
    let kef = r.u8();
    r.skip(3); // translation / rotation / scale formats (per track for AKF_PerTrackCompression)
    let bs_size = r.s32();
    let n_off = r.s32();
    let n_soff = r.s32();
    r.s32(); // StripSize
    if r.bad || n_off < 0 || n_soff < 0 {
        return err(format!("{}: does not decode", pkg_path));
    }
    if kef != AKF_PER_TRACK_COMPRESSION {
        return err(format!("{}: key encoding {} ({}) not implemented", pkg_path, kef, codec));
    }
    let (n_off, n_soff) = (n_off as usize, n_soff as usize);
    if stream.len() < 4 * (n_off + n_soff) || stream.len() - 4 * (n_off + n_soff) != bs_size as usize {
        return err(format!("{}: byte stream {}, expected {} + offsets", pkg_path, stream.len(), bs_size));
    }
    if 2 * table.len() > n_off {
        return err(format!("{}: {} tracks, {} offsets", pkg_path, table.len(), n_off));
    }
    let keys = &stream[4 * (n_off + n_soff)..];
    let mut tracks = Vec::with_capacity(table.len());
    for (ti, &bone) in table.iter().enumerate() {
        let to = le_i32(&stream, 8 * ti);
        let ro = le_i32(&stream, 8 * ti + 4);
        let so = if ti < n_soff { le_i32(&stream, 4 * n_off + 4 * ti) } else { -1 };
        let vec = |o: i32| -> Result<Option<VecTrack>> {
            if o == -1 {
                return Ok(None);
            }
            let (k, f) = track(keys, o, false, frames, pkg_path)?;
            Ok(Some(Channel { keys: k.into_iter().map(|q| [q[0], q[1], q[2]]).collect(), frames: f }))
        };
        let rot = if ro == -1 {
            None
        } else {
            let (k, f) = track(keys, ro, true, frames, pkg_path)?;
            Some(Channel { keys: k, frames: f })
        };
        tracks.push(Track { bone, pos: vec(to)?, rot, scale: vec(so)? });
    }
    Ok(AnimSequence {
        package: pkg.name.clone(),
        num_frames: frames,
        sequence_length: props::get_f64(&properties, "SequenceLength", 0.0) as f32,
        rate_scale: props::get_f64(&properties, "RateScale", 1.0) as f32,
        additive_anim_type: enum_tail(props::get_str(&properties, "AdditiveAnimType", "AAT_None")).to_string(),
        step: enum_tail(props::get_str(&properties, "Interpolation", "Linear")) == "Step",
        codec,
        curves,
        properties,
        tracks,
    })
}

/// (x, y, z) with w reconstructed (positive), FQuatFloat96NoW / FQuatFixed48NoW ... ToQuat
fn quat_w(x: f32, y: f32, z: f32) -> [f32; 4] {
    let w2 = 1.0 - x * x - y * y - z * z;
    [x, y, z, if w2 > 0.0 { w2.sqrt() } else { 0.0 }]
}

/// FAnimationCompression_PerTrackUtils fixed-48 per-component translation (DecodeFixed48_PerTrackComponent<7>)
fn fixed48c(v: u16) -> f32 {
    let off: i32 = (1 << (15 - 7)) - 1;
    (v as i32 - off) as f32 * (1.0 / (off >> 7) as f32)
}

/// One track's keys (rotation: quaternions; vectors in xyz with w = 0) and frame numbers
fn track(b: &[u8], o: i32, rot: bool, num_frames: i32, pkg: &str) -> Result<(Vec<[f32; 4]>, Vec<f32>)> {
    let bad = || crate::AssetError(format!("{}: track at {} beyond the byte stream", pkg, o));
    if o < 0 || o as usize + 4 > b.len() {
        return Err(bad());
    }
    let mut o = o as usize;
    let info = le_u32(b, o);
    o += 4;
    let fmt = info >> 28;
    let mask = (info >> 24) & 0xf;
    let n = (info & 0xff_ffff) as usize;
    // worst case per key: 12 bytes (+ 24 interval header + frame table 2/key); bound before reading
    if n > b.len() {
        return Err(bad());
    }
    let need = |o: usize, k: usize| -> Result<()> { if o + k > b.len() { Err(bad()) } else { Ok(()) } };
    let mut mins = [0f32; 3];
    let mut rng = [0f32; 3];
    if fmt == ACF_INTERVAL_FIXED32_NO_W {
        for c in 0..3 {
            if mask & (1 << c) != 0 {
                need(o, 8)?;
                mins[c] = le_f32(b, o);
                rng[c] = le_f32(b, o + 4);
                o += 8;
            }
        }
    }
    let mut keys = Vec::with_capacity(n);
    for _ in 0..n {
        let mut v = [0f32; 3];
        match fmt {
            ACF_NONE | ACF_FLOAT96_NO_W => {
                if rot || mask & 7 == 0 {
                    need(o, 12)?;
                    v = [le_f32(b, o), le_f32(b, o + 4), le_f32(b, o + 8)];
                    o += 12;
                } else {
                    for (c, vc) in v.iter_mut().enumerate() {
                        if mask & (1 << c) != 0 {
                            need(o, 4)?;
                            *vc = le_f32(b, o);
                            o += 4;
                        }
                    }
                }
            }
            ACF_FIXED48_NO_W => {
                for (c, vc) in v.iter_mut().enumerate() {
                    if mask & (1 << c) != 0 {
                        need(o, 2)?;
                        let u = le_u16(b, o);
                        o += 2;
                        *vc = if rot { (u as i32 - 32767) as f32 / 32767.0 } else { fixed48c(u) };
                    }
                }
            }
            ACF_INTERVAL_FIXED32_NO_W => {
                need(o, 4)?;
                let p = le_u32(b, o);
                o += 4;
                let hi = ((p >> 21) as i32 - 1023) as f32 / 1023.0;
                let mid = (((p & 0x001f_fc00) >> 10) as i32 - 1023) as f32 / 1023.0;
                let lo = ((p & 0x3ff) as i32 - 511) as f32 / 511.0;
                v = if rot {
                    [hi * rng[0] + mins[0], mid * rng[1] + mins[1], lo * rng[2] + mins[2]]
                } else {
                    [lo * rng[0] + mins[0], mid * rng[1] + mins[1], hi * rng[2] + mins[2]]
                };
            }
            ACF_FIXED32_NO_W if rot => {
                need(o, 4)?;
                let p = le_u32(b, o);
                o += 4;
                v = [
                    ((p >> 21) as i32 - 1023) as f32 / 1023.0,
                    (((p & 0x001f_fc00) >> 10) as i32 - 1023) as f32 / 1023.0,
                    ((p & 0x3ff) as i32 - 511) as f32 / 511.0,
                ];
            }
            ACF_FLOAT32_NO_W if rot => {
                need(o, 4)?;
                let p = le_u32(b, o);
                o += 4;
                let ux = p >> 21;
                let uy = (p & 0x001f_fc00) >> 10;
                let uz = p & 0x3ff;
                let f11 = |u: u32| f32::from_bits(((((u >> 7) & 7) + 123) << 23) | (((u & 0x7f) | 32 * (u & 0xffff_fc00)) << 16));
                let f10 = |u: u32| f32::from_bits(((((u >> 6) & 7) + 123) << 23) | (((u & 0x3f) | 32 * (u & 0xffff_fe00)) << 17));
                v = [f11(ux), f11(uy), f10(uz)];
            }
            ACF_IDENTITY => {}
            _ => {
                return err(format!("{}: {} format {} not implemented", pkg, if rot { "rotation" } else { "vector" }, fmt));
            }
        }
        keys.push(if rot { quat_w(v[0], v[1], v[2]) } else { [v[0], v[1], v[2], 0.0] });
    }
    o = (o + 3) & !3;
    let mut times = Vec::new();
    if mask & 8 != 0 && n > 1 {
        let w = if num_frames < 256 { 1 } else { 2 };
        need(o, n * w)?;
        for k in 0..n {
            times.push(if w == 1 { b[o + k] as f32 } else { le_u16(b, o + 2 * k) as f32 });
        }
    }
    Ok((keys, times))
}

/// Time in seconds of key k of a channel of n keys: the shipped engine spreads NumKeys over SequenceLength (KeyPos =
/// RelativePos * (NumKeys - 1), AEFPerTrackCompressionCodec::GetBoneAtomRotation VA 0x142e6b330, see
/// game/anim/additive_clip.gd); time-keyed channels hold frame numbers over NumFrames - 1 intervals
pub fn key_time(frames: &[f32], n: usize, k: usize, num_frames: i32, length: f32) -> f32 {
    if !frames.is_empty() {
        return frames[k] / (num_frames - 1).max(1) as f32 * length;
    }
    if n <= 1 { 0.0 } else { k as f32 / (n - 1) as f32 * length }
}

/// (key0, key1, alpha) of a channel of n keys at RelativePos rp = time / SequenceLength, as
/// AEFPerTrackCompressionCodec::GetBoneAtomRotation (VA 0x142e6b330) searches: rp <= 0 -> key 0, >= 1 -> last key; no
/// time keys: Pos = rp * (NumKeys - 1), key0 = floor, key1 = min(key0 + 1, NumKeys - 1), alpha = Pos - key0; time keys
/// (frame numbers): FramePos = rp * (NumFrames - 1), key0 = the last key whose frame <= min(floor(FramePos),
/// NumFrames - 2), key1 = min(key0 + 1, NumKeys - 1), alpha = (FramePos - frame0) / max(frame1 - frame0, 1);
/// Interpolation Step -> alpha 0 (additive_clip.gd _keys). UNCONFIRMED: translation / scale take the rotation's search
/// (only the rotation's search was read off the exe; additive_clip.gd notes the same).
pub fn key_params(n: usize, frames: &[f32], rp: f32, num_frames: i32, step: bool) -> (usize, usize, f32) {
    if n <= 1 || rp <= 0.0 {
        return (0, 0, 0.0);
    }
    if rp >= 1.0 {
        return (n - 1, n - 1, 0.0);
    }
    let (k0, k1, mut a);
    if frames.is_empty() {
        let pos = rp * (n - 1) as f32;
        k0 = (pos.floor() as usize).min(n - 1);
        k1 = (k0 + 1).min(n - 1);
        a = pos - k0 as f32;
    } else {
        let fpos = rp * (num_frames - 1) as f32;
        let fi = (fpos.floor() as i32).min(num_frames - 2);
        let mut k = 0;
        for (i, &f) in frames.iter().enumerate().take(n) {
            if f as i32 <= fi {
                k = i;
            } else {
                break;
            }
        }
        k0 = k;
        k1 = (k0 + 1).min(n - 1);
        a = (fpos - frames[k0]) / ((frames[k1] as i32 - frames[k0] as i32).max(1)) as f32;
    }
    if step {
        a = 0.0;
    }
    (k0, k1, a)
}

fn normalize(q: [f32; 4]) -> [f32; 4] {
    let l = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if l > 0.0 { [q[0] / l, q[1] / l, q[2] / l, q[3] / l] } else { [0.0, 0.0, 0.0, 1.0] }
}

/// FQuat::FastLerp + Normalize as the decoder does it (0x142e6bf0e..0x142e6c0b7; additive_clip.gd fast_lerp_norm)
pub fn fast_lerp_norm(k0: [f32; 4], k1: [f32; 4], a: f32) -> [f32; 4] {
    let d = k0[0] * k1[0] + k0[1] * k1[1] + k0[2] * k1[2] + k0[3] * k1[3];
    let mut r = [0f32; 4];
    for i in 0..4 {
        r[i] = if d >= 0.0 { k0[i] * (1.0 - a) + k1[i] * a } else { k1[i] * a - k0[i] * (1.0 - a) };
    }
    normalize(r)
}

/// A bone's local transform parts at time t (UE space): rotation (FastLerp + normalize), translation and scale
/// (k0 + (k1 - k0) * alpha, GetBoneAtomTranslation VA 0x142e70770). Absent channels give None: the caller keeps the
/// reference pose (or the additive identity, see additive_clip.gd).
impl AnimSequence {
    pub fn sample(&self, track: &Track, t: f32) -> (Option<[f32; 4]>, Option<[f32; 3]>, Option<[f32; 3]>) {
        let rp = if self.sequence_length > 0.0 { t / self.sequence_length } else { 0.0 };
        let rot = track.rot.as_ref().filter(|c| !c.keys.is_empty()).map(|c| {
            let (k0, k1, a) = key_params(c.keys.len(), &c.frames, rp, self.num_frames, self.step);
            if a > 0.0 { fast_lerp_norm(c.keys[k0], c.keys[k1], a) } else { normalize(c.keys[k0]) }
        });
        let lerp = |c: &VecTrack| {
            let (k0, k1, a) = key_params(c.keys.len(), &c.frames, rp, self.num_frames, self.step);
            let (p, q) = (c.keys[k0], c.keys[k1]);
            [p[0] + (q[0] - p[0]) * a, p[1] + (q[1] - p[1]) * a, p[2] + (q[2] - p[2]) * a]
        };
        let pos = track.pos.as_ref().filter(|c| !c.keys.is_empty()).map(lerp);
        let scale = track.scale.as_ref().filter(|c| !c.keys.is_empty()).map(lerp);
        (rot, pos, scale)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uniform_key_times_span_num_keys_minus_one() {
        assert_eq!(key_time(&[], 5, 4, 30, 2.0), 2.0);
        assert_eq!(key_time(&[], 5, 2, 30, 2.0), 1.0);
        assert_eq!(key_time(&[], 1, 0, 30, 2.0), 0.0);
        // frame table: frame 29 of 30 frames = the end
        assert_eq!(key_time(&[0.0, 29.0], 2, 1, 30, 2.0), 2.0);
    }

    #[test]
    fn key_search_matches_additive_clip() {
        assert_eq!(key_params(5, &[], 0.5, 30, false), (2, 3, 0.0));
        let (k0, k1, a) = key_params(5, &[], 0.6, 30, false);
        assert_eq!((k0, k1), (2, 3));
        assert!((a - 0.4).abs() < 1e-5);
        assert_eq!(key_params(5, &[], 1.0, 30, false), (4, 4, 0.0));
        // frame table 0, 10, 29 over 30 frames, rp 0.5 -> FramePos 14.5 -> key 1, alpha 4.5 / 19
        let (k0, k1, a) = key_params(3, &[0.0, 10.0, 29.0], 0.5, 30, false);
        assert_eq!((k0, k1), (1, 2));
        assert!((a - 4.5 / 19.0).abs() < 1e-6);
        assert_eq!(key_params(3, &[0.0, 10.0, 29.0], 0.5, 30, true).2, 0.0);
    }

    #[test]
    fn fast_lerp_takes_short_path() {
        let a = [0.0, 0.0, 0.0, 1.0];
        let b = [0.0, 0.0, 0.0, -1.0];
        let r = fast_lerp_norm(a, b, 0.5);
        assert!((r[3].abs() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn fixed48_translation_component() {
        assert_eq!(fixed48c(255), 0.0);
        assert_eq!(fixed48c(256), 1.0);
    }
}

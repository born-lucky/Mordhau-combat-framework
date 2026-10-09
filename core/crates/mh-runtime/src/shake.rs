//! shake.rs - the game's camera shakes on the local player's camera (fidelity-audit r2).
//!
//! Engine side (UE 4.26 UMatineeCameraShake / UCameraShakeBase as compiled into the exe):
//! - UMatineeCameraShake ctor rva=0x2f28ac0 (disasm): OscillationBlendInTime 0.1 (+0x9c), OscillationBlendOutTime 0.2
//!   (+0xa0); OscillationDuration (+0x98) 0. FFOscillator {Amplitude, Frequency, InitialOffset, Waveform} at RotOscillation
//!   +0xa4/+0xb0/+0xbc, LocOscillation +0xc8/+0xd4/+0xe0, FOVOscillation +0xec (offsets read in UpdateShakeImpl).
//!   FFOscillator member defaults (Amplitude 0, Frequency 1, InitialOffset EOO_OffsetRandom, Waveform SineWave): UE 4.26
//!   CameraShake.h, UNCONFIRMED against the exe's struct ctor (every shake the duel uses sets InitialOffset OffsetZero).
//! - UMatineeCameraShake::StartShakeImpl rva=0x2f43f70: initial sine offsets (OffsetRandom: FRand x 2pi), time
//!   remaining = OscillationDuration, blend-in; a running single-instance shake is refreshed (UE 4.26 source; the restart
//!   branch UNCONFIRMED against the disasm).
//! - UMatineeCameraShake::UpdateShakeImpl rva=0x2f46160 (disasm: per-axis `ucomiss Amplitude, 0`, Waveform == 1 ->
//!   PerlinNoise1D 0x1418b67c0 else sinf 0x143e23603, pitch clamp +-89.9 [0x144971178]): time remaining, blend
//!   in/out weights (min of both), offset = Amplitude x sin(offset += dt x Frequency), x ShakeScale x blend weight.
//! - UCameraShakeBase::ApplyPlaySpace rva=0x2f2b980 (calls: 2x FRotationMatrix 0x140b7e6f0, FMatrix::Inverse 0x14093c590,
//!   Rotator 0x1418bdb00): UserDefined: Location += U.TransformVector(LocOffset); Rotation = (Cam x U^-1 x Off x U)
//!   (row-vector matrices; the product order is UE 4.26 CameraShakeBase.cpp, UNCONFIRMED against the inlined SIMD).
//!   FOV += FOV offset.
//! Game side: AMordhauCharacter::DoCameraShakeIfViewTarget rva=0x1538f50 -> UMordhauCameraComponent::
//! DoCameraShakeIfViewTarget rva=0x14ba4f0: only the view target; Scale x m.Headbob when ShakeType != 0 or m.Headbob > 1
//! (every call here passes ShakeType 0, so the scale is unchanged for m.Headbob <= 1); PlaySpace 2 = UserDefined.
//! Triggers (all for the view target only):
//! - attacker's hit: UStrikeMotion::PlayHitShake rva=0x166ac50 (UserPlaySpaceRot = Quat(CameraRotation1P) x
//!   Quat(0, 0, AngleTarget x 60; left moves -R - 180)), UAttackMotion::PlayHitShake (rotation CameraRotation1P), shake
//!   = AttackInfo.HitShake, once per attack (bHasPlayedHitShake); from OnDynamicParamChanged rva=0x1631060 DYN_HIT.
//! - defender's block: AMordhauCharacter::OnRep_NetBlock rva=0x155a4e0: BlockShakeEffect when Reason < World and
//!   !bIsStun, rotation CameraRotation1P.
//! - attacker's blocked motion: UBlockedMotion::OnBegin_Implementation rva=0x165dc20 (decomp 355-411): BlockedShakeEffect,
//!   or the blocked attack's AttackInfo.HitStopShake for Reason Hit; strikes add the roll offset as PlayHitShake.
//! - flinch: UFlinchMotion::OnBegin_Implementation rva=0x16606a0 (decomp 60-195): FlinchShakeEffect at scale
//!   MapRangeClamped(FlinchDurationModifier, FlinchDurationModifierToShakeScaleIn, Out), rotation Quat(CameraRotation1P)
//!   x Quat(0, FlinchYaw + 90, 0); FlinchDurationModifier = byte/255 x 1.5 + 0.5, FlinchYaw = byte/255 x 360 - 180.
//! - fall damage: AMordhauCharacter::OnTookDamage_Implementation rva=0x155be10 (decomp 5062-5080): FallDamageCameraShake.

use mordhau_core::ue::{FQuat, FVector};
use std::collections::HashMap;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Osc {
    pub amplitude: f32,
    pub frequency: f32,
    pub random_offset: bool,
    pub perlin: bool,
}

/// A UMatineeCameraShake Blueprint's oscillation fields
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShakeDef {
    pub path: String,
    pub duration: f32,
    pub blend_in: f32,
    pub blend_out: f32,
    /// pitch, yaw, roll
    pub rot: [Osc; 3],
    /// x, y, z
    pub loc: [Osc; 3],
    pub fov: Osc,
    pub single_instance: bool,
}

fn osc(v: Option<&serde_json::Value>) -> Osc {
    let g = |k: &str| v.and_then(|o| o.get(k));
    Osc {
        amplitude: g("Amplitude").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32,
        frequency: g("Frequency").and_then(|x| x.as_f64()).unwrap_or(1.0) as f32,
        random_offset: g("InitialOffset").and_then(|x| x.as_str()).is_none_or(|s| s.ends_with("OffsetRandom")),
        perlin: g("Waveform").and_then(|x| x.as_str()).is_some_and(|s| s.ends_with("PerlinNoise")),
    }
}

impl ShakeDef {
    pub fn from_defaults(path: &str, d: &serde_json::Map<String, serde_json::Value>) -> ShakeDef {
        let f = |k: &str, dflt: f32| d.get(k).and_then(|x| x.as_f64()).map(|x| x as f32).unwrap_or(dflt);
        let r = d.get("RotOscillation");
        let l = d.get("LocOscillation");
        ShakeDef {
            path: path.to_string(),
            duration: f("OscillationDuration", 0.0),
            blend_in: f("OscillationBlendInTime", 0.1),
            blend_out: f("OscillationBlendOutTime", 0.2),
            rot: [osc(r.and_then(|r| r.get("Pitch"))), osc(r.and_then(|r| r.get("Yaw"))), osc(r.and_then(|r| r.get("Roll")))],
            loc: [osc(l.and_then(|r| r.get("X"))), osc(l.and_then(|r| r.get("Y"))), osc(l.and_then(|r| r.get("Z")))],
            fov: osc(d.get("FOVOscillation")),
            single_instance: d.get("bSingleInstance").and_then(|x| x.as_bool()).unwrap_or(false),
        }
    }
}

/// One playing shake (UMatineeCameraShake instance state)
#[derive(Clone, Debug)]
pub struct Shake {
    pub def: ShakeDef,
    pub scale: f32,
    pub user_rot: FQuat,
    remaining: f32,
    blending_in: bool,
    blending_out: bool,
    t_in: f32,
    t_out: f32,
    rot_off: [f32; 3],
    loc_off: [f32; 3],
    fov_off: f32,
    pub finished: bool,
    /// ECameraShakePlaySpace CameraLocal (0): the offset applies in the current camera frame (head bob); else
    /// UserDefined with `user_rot`
    pub camera_local: bool,
}

/// the offset one update produces (camera-local values before ApplyPlaySpace)
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ShakeOut {
    pub loc: [f32; 3],
    /// pitch, yaw, roll
    pub rot: [f32; 3],
    pub fov: f32,
}

fn sample(o: &Osc, off: &mut f32, dt: f32) -> f32 {
    if o.amplitude != 0.0 {
        *off += dt * o.frequency;
        // PerlinNoise1D (Waveform 1) is not ported: no duel shake uses it (UNCONFIRMED stand-in: sine)
        o.amplitude * off.sin()
    } else {
        0.0
    }
}

impl Shake {
    pub fn start(def: ShakeDef, scale: f32, user_rot: FQuat, rnd: &mut dyn FnMut() -> f32) -> Shake {
        let mut s = Shake { def, scale, user_rot, remaining: 0.0, blending_in: false, blending_out: false, t_in: 0.0, t_out: 0.0, rot_off: [0.0; 3], loc_off: [0.0; 3], fov_off: 0.0, finished: false, camera_local: false };
        s.restart(scale, user_rot, rnd);
        s
    }
    /// StartShakeImpl rva=0x2f43f70
    pub fn restart(&mut self, scale: f32, user_rot: FQuat, rnd: &mut dyn FnMut() -> f32) {
        self.scale = scale;
        self.user_rot = user_rot;
        self.finished = false;
        let d = &self.def;
        if d.duration != 0.0 {
            if self.remaining > 0.0 {
                self.remaining = d.duration;
                if self.blending_out {
                    let frac = if d.blend_out > 0.0 { self.t_out / d.blend_out } else { 1.0 };
                    self.blending_out = false;
                    self.t_out = 0.0;
                    if d.blend_in > 0.0 {
                        self.blending_in = true;
                        self.t_in = d.blend_in * (1.0 - frac);
                    } else {
                        self.blending_in = false;
                        self.t_in = 0.0;
                    }
                }
            } else {
                let two_pi = 2.0 * std::f32::consts::PI;
                let mut init = |o: &Osc| if o.random_offset { rnd() * two_pi } else { 0.0 };
                self.rot_off = [init(&d.rot[0]), init(&d.rot[1]), init(&d.rot[2])];
                self.loc_off = [init(&d.loc[0]), init(&d.loc[1]), init(&d.loc[2])];
                self.fov_off = init(&d.fov);
                self.remaining = d.duration;
                if d.blend_in > 0.0 {
                    self.blending_in = true;
                    self.t_in = 0.0;
                }
            }
        } else {
            self.finished = true;
        }
    }
    /// UpdateShakeImpl rva=0x2f46160; `cam_pitch` = the POV's pitch (the +-89.9 clamp)
    pub fn update(&mut self, dt: f32, cam_pitch: f32) -> ShakeOut {
        let d = self.def.clone();
        if self.finished || self.scale <= 0.0 {
            return ShakeOut::default();
        }
        if self.remaining > 0.0 {
            self.remaining = (self.remaining - dt).max(0.0);
        }
        if self.blending_in {
            self.t_in += dt;
        }
        if self.blending_out {
            self.t_out += dt;
        }
        let mut done = false;
        if self.remaining == 0.0 {
            done = true;
        } else if self.remaining < 0.0 {
            // indefinite
        } else if self.remaining < d.blend_out {
            self.blending_out = true;
            self.t_out = d.blend_out - self.remaining;
        }
        if self.blending_in && self.t_in > d.blend_in {
            self.blending_in = false;
        }
        if self.blending_out && self.t_out > d.blend_out {
            self.t_out = d.blend_out;
            self.remaining = 0.0;
            done = true;
        }
        if done {
            self.finished = true;
            return ShakeOut::default();
        }
        let w_in = if self.blending_in { self.t_in / d.blend_in } else { 1.0 };
        let w_out = if self.blending_out { 1.0 - self.t_out / d.blend_out } else { 1.0 };
        let s = self.scale * w_in.min(w_out);
        let mut out = ShakeOut::default();
        if s > 0.0 {
            for i in 0..3 {
                out.loc[i] = sample(&d.loc[i], &mut self.loc_off[i], dt) * s;
            }
            for i in 0..3 {
                out.rot[i] = sample(&d.rot[i], &mut self.rot_off[i], dt) * s;
            }
            // pitch: FRotator::ClampAxis(ClampAngle(NormalizeAxis(pitch) + offset, -89.9, 89.9) - NormalizeAxis(pitch))
            let np = normalize_axis(cam_pitch);
            out.rot[0] = clamp_axis((np + out.rot[0]).clamp(-89.9, 89.9) - np);
            out.fov = s * sample(&d.fov, &mut self.fov_off, dt);
        }
        out
    }
}

fn normalize_axis(a: f32) -> f32 {
    let mut a = clamp_axis(a);
    if a > 180.0 {
        a -= 360.0;
    }
    a
}
fn clamp_axis(a: f32) -> f32 {
    let mut a = a % 360.0;
    if a < 0.0 {
        a += 360.0;
    }
    a
}

/// ApplyPlaySpace (UserDefined): the shaken camera rotation (UE quaternion) and the location offset (UE cm, world)
pub fn apply_play_space(cam: FQuat, user: FQuat, o: &ShakeOut) -> (FQuat, FVector) {
    let off = FQuat::from_rotator(o.rot[0], o.rot[1], o.rot[2]);
    // row-vector Cam x U^-1 x Off x U  ==  quaternion U * Off * U^-1 * Cam
    let q = user.mul(off).mul(user.inverse()).mul(cam);
    (q, user.rotate(FVector::new(o.loc[0], o.loc[1], o.loc[2])))
}

/// The local camera's playing shakes (UCameraModifier_CameraShake::ActiveShakes)
#[derive(bevy::prelude::Resource, Default)]
pub struct CameraShakes {
    /// m.Headbob (UserSettings; None = SetToDefaults' 1): DoCameraShakeIfViewTarget rva=0x14ba4f0 scales ShakeType != 0
    /// shakes by it
    pub headbob: Option<f32>,
    pub active: Vec<Shake>,
    pub defs: HashMap<String, Option<ShakeDef>>,
    pub started: Vec<String>,
    rng: u32,
}

impl CameraShakes {
    fn rand(&mut self) -> f32 {
        // FMath::FRand stand-in (the shakes the duel plays use OffsetZero, so it is never read)
        self.rng = self.rng.wrapping_mul(1103515245).wrapping_add(12345);
        (self.rng >> 8) as f32 / 16777216.0
    }
    /// UCameraModifier_CameraShake::AddCameraShake rva=0x2f2a180: a bSingleInstance class restarts its running instance
    pub fn add(&mut self, def: ShakeDef, scale: f32, user_rot: FQuat) {
        if scale <= 0.0 {
            return;
        }
        self.started.push(def.path.clone());
        if self.started.len() > 32 {
            self.started.remove(0);
        }
        let mut r = self.rand();
        let mut rnd = move || {
            r = (r * 7.31 + 0.137).fract();
            r
        };
        if def.single_instance {
            if let Some(s) = self.active.iter_mut().find(|s| s.def.path == def.path) {
                s.restart(scale, user_rot, &mut rnd);
                return;
            }
        }
        self.active.push(Shake::start(def, scale, user_rot, &mut rnd));
    }
    /// all shakes on a POV (UE camera quaternion + location cm + horizontal FOV)
    pub fn apply(&mut self, dt: f32, cam: FQuat, cam_pitch: f32) -> (FQuat, FVector, f32) {
        let mut q = cam;
        let mut loc = FVector::new(0.0, 0.0, 0.0);
        let mut fov = 0.0;
        for s in &mut self.active {
            let o = s.update(dt, cam_pitch);
            // CameraLocal: Cam * Off and the location offset in the camera frame (= UserDefined with U = the camera)
            let u = if s.camera_local { q } else { s.user_rot };
            let (nq, l) = apply_play_space(q, u, &o);
            q = nq;
            loc = FVector::new(loc.x + l.x, loc.y + l.y, loc.z + l.z);
            fov += o.fov;
        }
        self.active.retain(|s| !s.finished);
        (q, loc, fov)
    }
}

/// UFlinchMotion: MapRangeClamped(dur, In, Out) (decomp UFlinchMotion.cpp 166-195; In / Out from BP_FlinchMotion CDO
/// over the ctor's (1, 1.5) / (1, 2), decomp 447-450)
pub fn flinch_scale(dur: f32, i: (f32, f32), o: (f32, f32)) -> f32 {
    let r = i.1 - i.0;
    let a = if r.abs() > 1e-8 { ((dur - i.0) / r).clamp(0.0, 1.0) } else if dur < i.1 { 0.0 } else { 1.0 };
    (o.1 - o.0) * a + o.0
}

/// UStrikeMotion::PlayHitShake / UBlockedMotion roll offset: AngleTarget x 60, left moves -R - 180
pub fn strike_roll(angle_target: f32, left: bool) -> f32 {
    let r = angle_target * 60.0;
    if left { -r - 180.0 } else { r }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_shake() -> ShakeDef {
        // BlockShake (Mordhau/Content/Mordhau/Blueprints/CameraShakes/BlockShake CDO)
        let j: serde_json::Value = serde_json::json!({"OscillationDuration": 0.25, "OscillationBlendInTime": 0.0, "OscillationBlendOutTime": 0.225,
            "RotOscillation": {"Pitch": {"Amplitude": 8.0, "Frequency": 8.0, "InitialOffset": "EOO_OffsetZero"}, "Yaw": {"Amplitude": 2.5, "Frequency": 8.0, "InitialOffset": "EOO_OffsetZero"}, "Roll": {"Frequency": 8.0, "InitialOffset": "EOO_OffsetZero"}},
            "LocOscillation": {"X": {"Amplitude": 15.0, "Frequency": 8.0, "InitialOffset": "EOO_OffsetZero"}},
            "FOVOscillation": {"InitialOffset": "EOO_OffsetZero"}, "bSingleInstance": true});
        ShakeDef::from_defaults("BlockShake", j.as_object().unwrap())
    }

    /// UpdateShakeImpl: no blend-in (0), duration 0.25, blend-out 0.225 -> at t = 0.1: remaining 0.15 < 0.225, blending
    /// out with t_out = 0.075, weight 1 - 0.075/0.225 = 2/3; pitch = 8 sin(8 x 0.1) x 2/3
    #[test]
    fn block_shake_curve() {
        let mut s = Shake::start(block_shake(), 1.0, FQuat::IDENTITY, &mut || 0.0);
        let o = s.update(0.1, 0.0);
        let w = 1.0 - 0.075 / 0.225;
        assert!((o.rot[0] - 8.0 * (0.8f32).sin() * w).abs() < 1e-4, "{:?}", o);
        assert!((o.loc[0] - 15.0 * (0.8f32).sin() * w).abs() < 1e-4);
        assert_eq!(o.rot[2], 0.0);
        let o = s.update(0.2, 0.0);
        assert_eq!(o, ShakeOut::default());
        assert!(s.finished);
    }

    #[test]
    fn single_instance_restarts_and_playspace() {
        let mut c = CameraShakes::default();
        c.add(block_shake(), 1.0, FQuat::IDENTITY);
        c.add(block_shake(), 1.0, FQuat::IDENTITY);
        assert_eq!(c.active.len(), 1);
        // identity play space: rotation offset applied as Off * Cam
        let o = ShakeOut { loc: [1.0, 0.0, 0.0], rot: [0.0, 10.0, 0.0], fov: 0.0 };
        let (q, l) = apply_play_space(FQuat::IDENTITY, FQuat::from_rotator(0.0, 90.0, 0.0), &o);
        assert!((l.y - 1.0).abs() < 1e-5 && l.x.abs() < 1e-5, "{:?}", l);
        let f = q.rotate(FVector::new(1.0, 0.0, 0.0));
        assert!((f.y - (10f32).to_radians().sin()).abs() < 1e-4, "{:?}", f);
        assert!((flinch_scale(1.22, (1.0, 1.22), (1.0, 2.5)) - 2.5).abs() < 1e-6);
        assert!((flinch_scale(0.5, (1.0, 1.22), (1.0, 2.5)) - 1.0).abs() < 1e-6);
        assert_eq!(strike_roll(0.5, true), -210.0);
    }
}

/// BP_MordhauCharacter: the character Blueprint whose CDO holds BlockShakeEffect / BlockedShakeEffect /
/// FlinchShakeEffect / FallDamageCameraShake (AMordhauCharacter +0x1068 / +0x1090 / +0x1098 / +0x10a0)
pub const CHARACTER_BP: &str = "Mordhau/Content/Mordhau/Blueprints/Characters/BP_MordhauCharacter";
/// the flinch motion Blueprint (FlinchDurationModifierToShakeScaleIn / Out)
pub const FLINCH_BP: &str = "Mordhau/Content/Mordhau/Blueprints/Motions/BP_FlinchMotion";

#[derive(Default)]
pub struct ShakeData {
    pub character: serde_json::Map<String, serde_json::Value>,
    pub flinch_in: (f32, f32),
    pub flinch_out: (f32, f32),
    pub weapons: HashMap<String, serde_json::Map<String, serde_json::Value>>,
    /// (fighter index, attack start time) whose hit shake played (bHasPlayedHitShake)
    pub played: Vec<(usize, u64)>,
}

fn v2(d: &serde_json::Map<String, serde_json::Value>, k: &str, dflt: (f32, f32)) -> (f32, f32) {
    let o = d.get(k);
    let g = |c: &str, x: f32| o.and_then(|o| o.get(c)).and_then(|v| v.as_f64()).map(|v| v as f32).unwrap_or(x);
    (g("X", dflt.0), g("Y", dflt.1))
}

impl CameraShakes {
    fn def(&mut self, rd: &mh_pak::Reader, class: &serde_json::Value) -> Option<ShakeDef> {
        let p = mh_assets::material::ue_pkg_path(class);
        if p.is_empty() {
            return None;
        }
        self.defs.entry(p.clone()).or_insert_with(|| Some(ShakeDef::from_defaults(&p, &rd.defaults(&p)))).clone()
    }

    /// The shake triggers of one sim event for the local view target `me` (see the module header for the exe rules).
    /// `cam1p` = CameraRotation1P (pitch, yaw) of the view target.
    pub fn on_event(&mut self, data: &mut ShakeData, rd: &mh_pak::Reader, sim: &dyn crate::sim::SimBackend, ev: &serde_json::Value, me: u32, cam1p: (f32, f32)) {
        if data.character.is_empty() {
            data.character = rd.defaults(CHARACTER_BP);
            let f = rd.defaults(FLINCH_BP);
            data.flinch_in = v2(&f, "FlinchDurationModifierToShakeScaleIn", (1.0, 1.5));
            data.flinch_out = v2(&f, "FlinchDurationModifierToShakeScaleOut", (1.0, 2.0));
        }
        let Some(w) = sim.combat() else { return };
        let Some(my_fi) = sim.fighter_index(me) else { return };
        let s = |k: &str| ev.get(k).and_then(|x| x.as_str()).unwrap_or("");
        let is_me = |k: &str| sim.id_of(s(k)) == Some(me);
        let cam = FQuat::from_rotator(cam1p.0, cam1p.1, 0.0);
        let null = serde_json::Value::Null;
        let attack_shake = |data: &mut ShakeData, mv: i64, field: &str| -> serde_json::Value {
            let Some(wp) = sim.weapon_path(me) else { return serde_json::Value::Null };
            let alt = w.fighters.get(my_fi).is_some_and(|f| f.alternate_mode);
            let d = data.weapons.entry(wp.clone()).or_insert_with(|| rd.defaults(&wp));
            // the FAttackInfo the attack motion copies (UAttackMotion.cpp 1486): Strike (0, 1) / Stab (2, 3) / Kick (4)
            let a = match mv {
                0 | 1 => "StrikeAttack",
                2 | 3 => "StabAttack",
                4 => "KickAttack",
                _ => return serde_json::Value::Null,
            };
            let a = if alt { format!("Second{a}") } else { a.to_string() };
            d.get(&a).and_then(|x| x.get(field)).cloned().unwrap_or(serde_json::Value::Null)
        };
        match s("kind") {
            // AMordhauCharacter::OnFootstep rva=0x1552330 (decomp AMordhauCharacter.cpp 9495-9590): the view target's
            // footstep plays Run*HeadBobShake when the anim instance's Velocity (UMordhauAnimInstance +0x57c) > 75, else
            // Walk*HeadBobShake when > 20; Limb (FeetBones: 0 RightFootstep, 1 LeftFootstep) != 0 -> the Left one;
            // DoCameraShakeIfViewTarget(Scale 1, PlaySpace CameraLocal, ShakeType 1 -> Scale x m.Headbob). The event's
            // 2D speed stands in for Velocity (UNCONFIRMED: NativeUpdateAnimation's exact Velocity source)
            "foot_landed" if is_me("who") => {
                let speed = ev.get("speed").and_then(|x| x.as_f64()).unwrap_or(0.0);
                if speed <= 20.0 {
                    return;
                }
                let left = ev.get("foot").and_then(|x| x.as_i64()) == Some(0);
                let field = match (speed > 75.0, left) {
                    (true, true) => "RunLeftHeadBobShake",
                    (true, false) => "RunRightHeadBobShake",
                    (false, true) => "WalkLeftHeadBobShake",
                    (false, false) => "WalkRightHeadBobShake",
                };
                let c = data.character.get(field).cloned().unwrap_or(null);
                let Some(def) = self.def(rd, &c) else { return };
                let scale = self.headbob.unwrap_or(1.0);
                self.add(def, scale, cam);
                if let Some(sh) = self.active.last_mut() {
                    sh.camera_local = true;
                }
            }
            // the attacker's DYN_HIT -> PlayHitShake (once per attack)
            "hit" if is_me("attacker") => {
                let Some(m) = w.cur_m(my_fi) else { return };
                let Some(a) = m.attack() else { return };
                let key = (my_fi, m.start_time.to_bits());
                if data.played.contains(&key) {
                    return;
                }
                data.played.push(key);
                if data.played.len() > 16 {
                    data.played.remove(0);
                }
                let (mv, at, strike) = (a.mv, a.angle_target as f32, a.native == "UStrikeMotion");
                let c = attack_shake(data, mv, "HitShake");
                let Some(def) = self.def(rd, &c) else { return };
                let rot = if strike { cam.mul(FQuat::from_rotator(0.0, 0.0, strike_roll(at, crate::input::is_left(mv)))) } else { cam };
                self.add(def, 1.0, rot);
            }
            // the defender's OnRep_NetBlock: Reason < World (Parry, Chamber) and !bIsStun -> BlockShakeEffect
            "parry" | "active_parry" | "chamber" if is_me("victim") => {
                let stun = w.fighter_index(s("attacker")).and_then(|a| w.cur_m(a)).and_then(|m| m.blocked()).is_some_and(|b| b.b_is_stun);
                if stun {
                    return;
                }
                let c = data.character.get("BlockShakeEffect").cloned().unwrap_or(null);
                if let Some(def) = self.def(rd, &c) {
                    self.add(def, 1.0, cam);
                }
            }
            "motion" if is_me("who") => match s("motion") {
                "Blocked" => {
                    let Some(m) = w.cur_m(my_fi) else { return };
                    let Some(b) = m.blocked() else { return };
                    let from = b.from_attack.map(|id| w.m(my_fi, id));
                    let at = from.and_then(|f| f.attack()).map(|a| a.angle_target as f32).unwrap_or(0.0);
                    let c = if b.reason == mordhau_core::combat::enums::br::HIT {
                        attack_shake(data, b.from_move, "HitStopShake")
                    } else {
                        data.character.get("BlockedShakeEffect").cloned().unwrap_or(null)
                    };
                    let Some(def) = self.def(rd, &c) else { return };
                    let roll = if b.from_move == 0 || b.from_move == 1 { strike_roll(at, b.from_move == 1) } else { 0.0 };
                    self.add(def, 1.0, cam.mul(FQuat::from_rotator(0.0, 0.0, roll)));
                }
                "Flinch" => {
                    let n = w.fighters[my_fi].net;
                    let dur = (n.param2 as f32 * 0.003921569).clamp(0.0, 1.0) * 1.5 + 0.5;
                    let yaw = (n.param0 as f32 * 0.003921569).clamp(0.0, 1.0) * 360.0 - 180.0;
                    let c = data.character.get("FlinchShakeEffect").cloned().unwrap_or(null);
                    if let Some(def) = self.def(rd, &c) {
                        self.add(def, flinch_scale(dur, data.flinch_in, data.flinch_out), cam.mul(FQuat::from_rotator(0.0, yaw + 90.0, 0.0)));
                    }
                }
                _ => {}
            },
            "fall_damage" if is_me("who") => {
                let c = data.character.get("FallDamageCameraShake").cloned().unwrap_or(null);
                if let Some(def) = self.def(rd, &c) {
                    self.add(def, 1.0, cam);
                }
            }
            _ => {}
        }
    }
}

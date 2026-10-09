//! grip r2 (state/proofs/grip.md B3/B4, O3): the held weapon's grip per weapon mode, and the mode switch's "virtual
//! reparent" of the weapon mesh.
//!
//! - **Per-mode grip.** AMordhauEquipment::SwitchMode_Implementation rva=0x156e280 (decomp AMordhauEquipment.cpp
//!   3150-3300) swaps RotationOffset with SecondRotationOffset, GripLocationLocal with SecondGripLocationLocal,
//!   bUseEquippedOffset with bSecondUseEquippedOffset, and bIsRightHanded / bIsTwoHanded with their Second* fields.
//!   AMordhauCharacter::SwitchModeAndReAttach (decomp AMordhauCharacter.cpp 24000-24012) then re-runs
//!   ComputeGrippedTransform rva=0x14b70f0 and re-attaches. EquippedOffset and RightHandEquipOffset are not swapped.
//!   A weapon in its alternate mode is therefore gripped with the CDO's Second* values ([`GripMode`] index 1).
//! - **The switch.** UEquipmentModeSwitchMotion: OnBegin rva=0x165fbb0, OnTick rva=0x1667e00,
//!   PerformVirtualReparentTrickery rva=0x166a520, FinishSwitch rva=0x165b4d0, OnLeave rva=0x16659a0. During the motion
//!   the weapon's SkeletalMeshComponent is placed in world space ([`GripPosed::mesh_world`]):
//!   - stage 0: the old mode's grip on the old socket;
//!   - stage 1: a lerp / slerp from the old grip, carried by an anchor (the LeftHand for switch type 0, else the new
//!     socket) since the stage began, to the new grip;
//!   - stage 2 and after the motion: the new grip on the new socket, which is the re-attached actor
//!     (OnLeave resets the mesh to DefaultMeshRelativeTransform, identity).

use crate::trace::{Geometry, Posed, WeaponGeo};
use mordhau_core::ue::{normalized_time, FQuat, FTransform, FVector};

/// One weapon mode's grip fields (the CDO values; index 0 = primary, 1 = the Second* fields)
#[derive(Clone, Debug, Default)]
pub struct GripMode {
    /// RotationOffset +0xc20 / SecondRotationOffset +0xc2c (pitch, yaw, roll)
    pub rotation_offset: [f32; 3],
    /// GripLocationLocal +0xc38 / SecondGripLocationLocal +0xc44
    pub grip_location_local: FVector,
    /// bUseEquippedOffset / bSecondUseEquippedOffset
    pub use_equipped_offset: bool,
    /// bIsRightHanded / bSecondIsRightHanded (ctor both true, decomp AMordhauEquipment.cpp 1653-1654)
    pub right_handed: bool,
    /// bIsTwoHanded / bSecondIsTwoHanded (ctor false, decomp 1655)
    pub two_handed: bool,
    /// ModeSwitchAnimation / SecondModeSwitchAnimation is set
    pub has_mode_switch_animation: bool,
}

/// UEquipmentModeSwitchMotion ctor rva=0x1646800: Stage1Duration 0.25, Stage2Duration 0.15 (BP_MordhauCharacter's
/// motion list names the native class EquipmentModeSwitchMotion itself, so no Blueprint overrides them)
pub const STAGE1_DURATION: f64 = 0.25;
pub const STAGE2_DURATION: f64 = 0.15;

/// A running mode switch (UEquipmentModeSwitchMotion's fields)
#[derive(Clone, Debug)]
pub struct Switch {
    pub start: f64,
    /// the mode being switched to (bIsSwitchingToAlt)
    pub to_alt: bool,
    /// SwitchType: 0 hand-over through the left hand (same hand, both two-handed, ModeSwitchAnimation set); 1 same
    /// hand otherwise; 2 / 3 the hand changes (3: the mode being left is right-handed)
    pub kind: u8,
    pub stage: u8,
    pub first_stage_end: f64,
    pub second_stage_end: f64,
    /// VirtualReparentLocation / VirtualReparentRotation: the old grip in the anchor's frame, taken at stage 1
    pub reparent: Option<FTransform>,
}

/// Per fighter: which weapon / mode the held weapon is gripped in, the running switch, and the mesh override
#[derive(Clone, Debug, Default)]
pub struct GripPosed {
    pub weapon: String,
    pub alt: bool,
    pub switch: Option<Switch>,
    /// the weapon mesh's world transform while a switch overrides it (PerformVirtualReparentTrickery)
    pub mesh_world: Option<FTransform>,
    /// the fighter is drawn in first person (Sim::refresh_pose sets it from FighterAnim::first_person)
    pub first_person: bool,
}

fn socket(m: &GripMode) -> &'static str {
    if m.right_handed {
        "RightWeapon"
    } else {
        "LeftWeapon"
    }
}

/// UEquipmentSystemComponent::ComputeGrippedTransform rva=0x14b70f0 with mode `alt`'s fields on the socket transform
/// `s`; `left` = bIsLeft (the LeftHand / LeftWeapon socket: rva=0x14b7990)
pub fn gripped_mode(geo: &Geometry, s: &FTransform, w: &WeaponGeo, alt: bool, left: bool) -> FTransform {
    let Some(modes) = &w.grip_modes else {
        // a WeaponGeo built without the mode table (tests): the primary fields
        return legacy(geo, s, w, left);
    };
    let m = &modes[alt as usize];
    // decomp 74-79 / 208-447: bUseEquippedOffset -> FTransform::Multiply(EquippedOffset, S)
    if m.use_equipped_offset {
        return w.equipped_offset_xf.then(s);
    }
    // decomp 80-103: bIsLeft flips the pitch / yaw sign bits and negates the roll
    let r = m.rotation_offset;
    let rot = if left { [-r[0], -r[1], -r[2]] } else { r };
    let pitch = if left { geo.grip_pitch_left } else { geo.grip_pitch_right };
    crate::pose::gripped(s, w.right_hand_equip_offset, rot, pitch, m.grip_location_local)
}

fn legacy(geo: &Geometry, s: &FTransform, w: &WeaponGeo, left: bool) -> FTransform {
    if let Some(eo) = &w.equipped_offset {
        return eo.then(s);
    }
    let r = w.rotation_offset;
    let rot = if left { [-r[0], -r[1], -r[2]] } else { r };
    let pitch = if left { geo.grip_pitch_left } else { geo.grip_pitch_right };
    crate::pose::gripped(s, w.right_hand_equip_offset, rot, pitch, w.grip_location_local)
}

/// The steady grip: mode `alt` on its own socket. None when the skeleton lacks the socket bone.
pub fn steady(geo: &Geometry, p: &Posed, w: &WeaponGeo, alt: bool) -> Option<FTransform> {
    let right = match &w.grip_modes {
        Some(m) => m[alt as usize].right_handed,
        None => w.right_handed,
    };
    let sock = if right { "RightWeapon" } else { "LeftWeapon" };
    let s = geo.bone_world(p, geo.skeleton.find(sock)?);
    let g = gripped_mode(geo, &s, w, alt, !right);
    // REVERTED to off by default (user 15:46: the turned sword was "not in the hand"): MH_GRIP_ROLL_1P=<deg> turns the 1P weapon
    // is turned 90 deg about its blade axis (weapon-local +Z). Every reference frame (state/reference/fp 2H/1H idle:
    // cIxl_45.60, 8BAYa_37.30, 9y-bl_30.00, okWx_30.00) shows the guard across the top of the fist, which the exe
    // transform alone gives only in third person (the 3P bot holds it edge-forward, as in the game). Most likely the
    // real 1P clips roll the RightWeapon bone (or a 1P-only grip offset) that our port doesn't reproduce yet.
    // Comparison: state/gauntlet/fpfix/roll.png, bot.png. MH_GRIP_ROLL=<deg> overrides it (any perspective) for tests;
    // MH_GRIP_ROLL_1P=0 disables the 1P turn.
    let fp_roll = std::env::var("MH_GRIP_ROLL_1P").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0);
    let roll = std::env::var("MH_GRIP_ROLL").ok().and_then(|v| v.parse::<f32>().ok()).or(if p.grip.first_person { Some(fp_roll) } else { None });
    if let Some(d) = roll.filter(|d| *d != 0.0) {
        let h = d.to_radians() * 0.5;
        let q = FQuat { x: 0.0, y: 0.0, z: h.sin(), w: h.cos() };
        return Some(FTransform::new(q, FVector::ZERO).then(&g));
    }
    Some(g)
}

fn bone(geo: &Geometry, p: &Posed, name: &str) -> Option<FTransform> {
    Some(geo.bone_world(p, geo.skeleton.find(name)?))
}

/// FQuat::Slerp_NotNormalized (UE4 engine source, UnrealMath.cpp; not disassembled: UNCONFIRMED against the binary)
/// followed by the normalisation PerformVirtualReparentTrickery applies to its result
pub fn slerp(a: FQuat, b: FQuat, t: f32) -> FQuat {
    let raw = a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w;
    let cosom = raw.abs();
    let (s0, mut s1) = if cosom < 0.9999 {
        let omega = cosom.acos();
        let inv = 1.0 / omega.sin();
        (((1.0 - t) * omega).sin() * inv, (t * omega).sin() * inv)
    } else {
        (1.0 - t, t)
    };
    if raw < 0.0 {
        s1 = -s1;
    }
    let q = [s0 * a.x + s1 * b.x, s0 * a.y + s1 * b.y, s0 * a.z + s1 * b.z, s0 * a.w + s1 * b.w];
    let n = (q[0] * q[0] + q[1] * q[1] + q[2] * q[2] + q[3] * q[3]).sqrt();
    if n < 1e-4 {
        return FQuat::IDENTITY;
    }
    FQuat::new(q[0] / n, q[1] / n, q[2] / n, q[3] / n)
}

/// Grip override exists only for the current UEquipmentModeSwitchMotion. OnLeave16659a0
/// resets the mesh immediately; a direct mode swap simply reattaches with the current steady grip.
pub fn update(geo: &Geometry, p: &mut Posed, weapon: &str, alt: bool, now: f64, motion: Option<(f64, bool)>) {
    let Some(w) = geo.weapons.get(weapon) else {
        p.grip = GripPosed { weapon: weapon.to_string(), alt, ..Default::default() };
        return;
    };
    let mut g = std::mem::take(&mut p.grip);
    if g.weapon != weapon { g = GripPosed { weapon: weapon.to_string(), alt, ..Default::default() }; }
    match motion {
        Some((t0, to_alt)) if g.switch.as_ref().map(|s| (s.start, s.to_alt)) != Some((t0, to_alt)) => {
            g.switch = w.grip_modes.as_ref().map(|m| begin(m, !to_alt, to_alt, t0));
        }
        None => g.switch = None,
        _ => {}
    }
    g.alt = alt;
    g.mesh_world = None;
    if let (Some(sw), Some(modes)) = (g.switch.as_mut(), w.grip_modes.as_ref()) {
        g.mesh_world = tick(geo, p, w, modes, sw, now);
        // Keep stage2 identity until OnLeave; Type1 EndTime is later than SecondStageEnd.
    }
    p.grip = g;
}

/// OnBegin_Implementation rva=0x165fbb0 (decomp UEquipmentModeSwitchMotion.cpp 919-985): the stage times and the
/// switch type. `from` is the mode being left.
fn begin(m: &[GripMode; 2], from: bool, to_alt: bool, now: f64) -> Switch {
    let (o, n) = (&m[from as usize], &m[to_alt as usize]);
    let mut first = now + STAGE1_DURATION;
    let mut second = first + STAGE2_DURATION;
    let kind = if o.right_handed == n.right_handed {
        // the pre-swap ModeSwitchAnimation (the mode being left) and both bIsTwoHanded
        if o.has_mode_switch_animation && o.two_handed && n.two_handed {
            0
        } else {
            // SwitchType 1: FirstStageEnd = 0, SecondStageEnd -= Stage1Duration
            second = (now - first) + second;
            first = 0.0;
            1
        }
    } else if o.right_handed {
        3
    } else {
        2
    };
    Switch { start: now, to_alt, kind, stage: 0, first_stage_end: first, second_stage_end: second, reparent: None }
}

/// OnTick_Implementation rva=0x1667e00 (stage changes, VirtualReparent capture; decomp 662-848) then
/// PerformVirtualReparentTrickery rva=0x166a520 (decomp 4-374): the weapon mesh's world transform this frame
fn tick(geo: &Geometry, p: &Posed, w: &WeaponGeo, m: &[GripMode; 2], sw: &mut Switch, now: f64) -> Option<FTransform> {
    let (old_alt, new_alt) = (!sw.to_alt, sw.to_alt);
    let (o, n) = (&m[old_alt as usize], &m[new_alt as usize]);
    let old_grip = || -> Option<FTransform> {
        let s = bone(geo, p, socket(o))?;
        Some(gripped_mode(geo, &s, w, old_alt, !o.right_handed))
    };
    // Stage 0 -> 1 (FirstStageEnd <= now && now != FirstStageEnd): VirtualReparent = the old grip in the anchor's frame
    if sw.stage == 0 && sw.first_stage_end < now {
        sw.stage = 1;
        let anchor = if sw.kind == 0 { bone(geo, p, "LeftHand") } else { bone(geo, p, socket(n)) };
        if let (Some(a), Some(g)) = (anchor, old_grip()) {
            sw.reparent = Some(g.then(&a.inverse()));
        }
    }
    // Stage 1 -> 2
    if sw.stage == 1 && sw.second_stage_end < now {
        sw.stage = 2;
    }
    match sw.stage {
        0 => old_grip(),
        1 => {
            let (anchor, target) = if sw.kind == 0 {
                // ComputeGrippedTransform(LeftHand, bIsLeft = true, the new mode's fields)
                let a = bone(geo, p, "LeftHand")?;
                (a, gripped_mode(geo, &a, w, new_alt, true))
            } else {
                let a = bone(geo, p, socket(n))?;
                (a, gripped_mode(geo, &a, w, new_alt, !n.right_handed))
            };
            let from = if sw.kind == 1 { sw.start } else { sw.first_stage_end };
            let t = normalized_time(from, sw.second_stage_end, now) as f32;
            let start = sw.reparent.as_ref()?.then(&anchor);
            let l = start.loc + (target.loc - start.loc).scale(t as f64);
            Some(FTransform::new(slerp(start.rot, target.rot, t), l))
        }
        _ => None,
    }
}

/// The class defaults of both modes (Reader::defaults: the Blueprint chain merged over the ctor values)
pub fn modes_from_cdo(d: &serde_json::Map<String, serde_json::Value>) -> [GripMode; 2] {
    let f = |v: &serde_json::Value, k: &str| v.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
    let rot = |k: &str, dflt: [f32; 3]| d.get(k).map(|v| [f(v, "Pitch"), f(v, "Yaw"), f(v, "Roll")]).unwrap_or(dflt);
    let vec = |k: &str| d.get(k).map(|v| FVector::new(f(v, "X"), f(v, "Y"), f(v, "Z"))).unwrap_or(FVector::ZERO);
    let b = |k: &str, dflt: bool| d.get(k).and_then(|v| v.as_bool()).unwrap_or(dflt);
    let set = |k: &str| d.get(k).map(|v| !v.is_null() && v.get("ObjectPath").and_then(|p| p.as_str()).map(|p| !p.is_empty()).unwrap_or(false)).unwrap_or(false);
    [
        GripMode {
            // ctor (-15, 15, 0), decomp AMordhauEquipment.cpp 1661-1663
            rotation_offset: rot("RotationOffset", [-15.0, 15.0, 0.0]),
            grip_location_local: vec("GripLocationLocal"),
            use_equipped_offset: b("bUseEquippedOffset", false),
            right_handed: b("bIsRightHanded", true),
            two_handed: b("bIsTwoHanded", false),
            has_mode_switch_animation: set("ModeSwitchAnimation"),
        },
        GripMode {
            // not set by the ctor (zero); BP_MordhauWeapon sets (-15, 15, 0)
            rotation_offset: rot("SecondRotationOffset", [0.0, 0.0, 0.0]),
            grip_location_local: vec("SecondGripLocationLocal"),
            use_equipped_offset: b("bSecondUseEquippedOffset", false),
            right_handed: b("bSecondIsRightHanded", true),
            two_handed: b("bSecondIsTwoHanded", false),
            has_mode_switch_animation: set("SecondModeSwitchAnimation"),
        },
    ]
}

/// RightWeaponBoneCosmeticTransform(1P) of the mode in use (SwitchMode swaps the Second* ones in; NativeUpdateAnimation
/// rva=0x1501930 decomp 2919-2933 reads the 1P one in first person): (rotator, translation), identity when absent
pub fn cosmetic(d: &serde_json::Map<String, serde_json::Value>, alt: bool, first_person: bool) -> ((f32, f32, f32), FVector) {
    let key = format!("{}RightWeaponBoneCosmeticTransform{}", if alt { "Second" } else { "" }, if first_person { "1P" } else { "" });
    let Some(v) = d.get(&key) else { return ((0.0, 0.0, 0.0), FVector::ZERO) };
    let g = |o: &serde_json::Value, k: &str, z: f64| o[k].as_f64().unwrap_or(z) as f32;
    let r = &v["Rotation"];
    let q = if r.is_null() { FQuat::IDENTITY } else { FQuat::new(g(r, "X", 0.0), g(r, "Y", 0.0), g(r, "Z", 0.0), g(r, "W", 1.0)) };
    let t = &v["Translation"];
    (mordhau_core::combat::geometry::quat_rotator(q), FVector::new(g(t, "X", 0.0), g(t, "Y", 0.0), g(t, "Z", 0.0)))
}

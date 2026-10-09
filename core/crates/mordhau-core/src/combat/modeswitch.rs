//! UEquipmentModeSwitchMotion (fp-anim r3, authorized as the owner for this change by the orchestrator 2026-10-06;
//! proof: state/proofs/fp_anim_items.md Round 2, item 4). The weapon-mode switch is a motion: it blocks attacks while it
//! runs (bCanAttack false) and swaps the weapon mode (AMordhauCharacter::SwitchModeAndReAttach ->
//! AMordhauEquipment::SwitchMode_Implementation) 0.15 s after it starts, not at the request. Exe sources
//! (extract/native/decomp/UEquipmentModeSwitchMotion.cpp):
//!   ctor rva=0x1646800 (decomp 1044-1058): UMordhauMotion ctor + bCanAttack false, bDisablesAtmospherics true,
//!     Stage1Duration 0.25, Stage2Duration 0.15. No Blueprint subclass: BP_MordhauCharacter Motions[11] is the native
//!     class EquipmentModeSwitchMotion (extract/json BP_MordhauCharacter.json).
//!   OnBegin_Implementation rva=0x165fbb0 (decomp 865-990, disasm 0x14165fdd1..0x14165fe6a): FirstStageEnd = StartTime +
//!     Stage1Duration, SecondStageEnd = FirstStageEnd + Stage2Duration, EndTime = SecondStageEnd; bIsSwitchingToAlt =
//!     the request's mode; SwitchType: bIsRightHanded (+0x569) != bSecondIsRightHanded (+0x56a) -> 2 + (the target
//!     mode's right-handedness); else no ModeSwitchAnimation (+0x8f0), !bIsTwoHanded (+0x56b) or !bSecondIsTwoHanded
//!     (+0x56c) -> 1 with SecondStageEnd -= Stage1Duration and EndTime += Stage1Duration (disasm: xmm2 = FirstStageEnd -
//!     StartTime is ADDED to EndTime), else 0.
//!   OnTick_Implementation rva=0x1667e00 (decomp 702-705): TimeSeconds > StartTime + 0.15 and !bHasFinishedSwitch ->
//!     FinishSwitch rva=0x165b4d0 (authority, the equipment still held, CheckCanEquipAlt -> SwitchModeAndReAttach); the
//!     stage changes at FirstStageEnd / SecondStageEnd (offhand IK and the virtual reparent: animation / grip side).
//!   OnLeave_Implementation rva=0x16659a0: FinishSwitch if not done yet.
//! Every alternate-mode equipment in the paks has both ModeSwitchAnimation and SecondModeSwitchAnimation set (scan of
//! extract/json Blueprints/Equipment, fp-anim r3), so "no montage" is taken as never true. The net path (FNetMotion type
//! of the motion) is not modelled: the switch starts locally (UNCONFIRMED for a client).

use super::motion::MotionKind;
use super::world::World;
use crate::data::{MotionBaseDef, MotionDef};
use std::rc::Rc;

/// UEquipmentModeSwitchMotion ctor rva=0x1646800: Stage1Duration (+0xd8) / Stage2Duration (+0xdc)
pub const STAGE1_DURATION: f64 = 0.25;
pub const STAGE2_DURATION: f64 = 0.15;
/// OnTick_Implementation decomp 702: FinishSwitch once TimeSeconds > StartTime + 0.15
pub const FINISH_SWITCH_AFTER: f64 = 0.15;

#[derive(Clone, Debug, Default)]
pub struct ModeSwitchMotion {
    pub b_is_switching_to_alt: bool,
    pub switch_type: i64,
    pub first_stage_end: f64,
    pub second_stage_end: f64,
    pub stage: i64,
    pub b_has_finished_switch: bool,
}

/// the native class defaults: UMordhauMotion ctor rva=0x1647f80 (decomp UMordhauMotion.cpp 195-213: bIsFlinchable,
/// bCanAttack, bCanBlock true, MovementRestriction 0, SpeedFactor / BackpedalSpeedFactor 1) + this class's ctor
/// (bCanAttack false)
pub fn mode_switch_def() -> Rc<MotionDef> {
    Rc::new(MotionDef {
        base: MotionBaseDef {
            rec: "Base".into(),
            path: String::new(),
            native: "UEquipmentModeSwitchMotion".into(),
            b_is_flinchable: true,
            b_can_attack: false,
            b_can_block: true,
            b_can_emote: false,
            b_blocks_regen: false,
            speed_factor: 1.0,
            backpedal_speed_factor: 1.0,
            shield_wall_speed_factor: 1.0,
            movement_restriction: 0,
        },
        ..Default::default()
    })
}

impl World {
    pub fn mode_switch(&self, fi: usize, id: super::motion::MotionId) -> Option<&ModeSwitchMotion> {
        if let MotionKind::ModeSwitch(m) = &self.m(fi, id).k { Some(m) } else { None }
    }

    /// AMordhauWeapon::OnRequestModeSwitch_Implementation rva=0x16327a0 (bHasAlternateMode branch): CheckCanEquipAlt,
    /// CanInitiateMotion(UEquipmentModeSwitchMotion, true), then the motion (AssignNetMotion; local here)
    pub fn switch_mode(&mut self, fi: usize) {
        let f = &self.fighters[fi];
        if f.weapon.is_none() || !self.check_can_equip_alt(fi, f.weapon_equip.as_deref()) {
            return;
        }
        if !self.can_initiate_motion(fi, "EquipmentModeSwitch", true) {
            return;
        }
        let to_alt = !self.fighters[fi].alternate_mode;
        if self.net.is_some() {
            // a networked machine: AssignNetMotion (mh-net replicates it; begin_net_motion creates the motion)
            self.assign(fi, super::system::NetMotion::new(super::enums::net::EQUIPMENT_MODE_SWITCH, to_alt as i64, 0, 0, 0));
            return;
        }
        let id = self.alloc_motion(fi, super::motion::Motion::new(mode_switch_def(), MotionKind::ModeSwitch(Box::new(ModeSwitchMotion { b_is_switching_to_alt: to_alt, ..Default::default() }))));
        self.change(fi, id);
    }

    /// UEquipmentModeSwitchMotion::OnBegin_Implementation rva=0x165fbb0 (the timing and SwitchType)
    pub(crate) fn mode_switch_on_begin(&mut self, fi: usize, id: super::motion::MotionId) {
        let q = self.qf();
        let e = self.fighters[fi].weapon_equip.clone();
        let alt_now = self.fighters[fi].alternate_mode;
        let m = self.mm(fi, id);
        let start = m.start_time;
        let fse = q(start + STAGE1_DURATION);
        let sse = q(fse + STAGE2_DURATION);
        m.end_time = sse;
        let mut end = sse;
        let (mut first, mut second, mut ty) = (fse, sse, 0);
        if let Some(e) = e.as_deref() {
            if e.b_is_right_handed != e.b_second_is_right_handed {
                // 2 + bIsRightHanded of the current mode (bIsSwitchingToAlt != bIsUsingAlternateMode holds at OnBegin,
                // the swap comes later; the EquipmentDef keeps the class defaults, so the current mode's field is the
                // Second* one while in the alternate mode)
                let rh = if alt_now { e.b_second_is_right_handed } else { e.b_is_right_handed };
                ty = if rh { 3 } else { 2 };
            } else if !e.b_is_two_handed || !e.b_second_is_two_handed {
                ty = 1;
                // disasm 0x14165fe25..0x14165fe6a: SecondStageEnd = (StartTime - FirstStageEnd) + SecondStageEnd,
                // EndTime = (FirstStageEnd - StartTime) + EndTime
                second = q(q(start - first) + second);
                end = q(q(first - start) + end);
                first = 0.0;
            }
        }
        m.end_time = end;
        if let MotionKind::ModeSwitch(s) = &mut m.k {
            s.switch_type = ty;
            s.first_stage_end = first;
            s.second_stage_end = second;
            s.stage = 0;
        }
    }

    /// UEquipmentModeSwitchMotion::OnTick_Implementation rva=0x1667e00 (gameplay part)
    pub(crate) fn mode_switch_on_tick(&mut self, fi: usize, id: super::motion::MotionId) {
        let now = self.now;
        let (start, finished, stage, fse, sse) = {
            let m = self.m(fi, id);
            let s = m.mode_switch().unwrap();
            (m.start_time, s.b_has_finished_switch, s.stage, s.first_stage_end, s.second_stage_end)
        };
        let q = self.qf();
        if q(start + FINISH_SWITCH_AFTER) < now && !finished {
            self.finish_mode_switch(fi, id);
        }
        let ms = self.mm(fi, id).mode_switch_mut().unwrap();
        if stage == 0 && fse < now {
            ms.stage = 1;
        }
        if ms.stage == 1 && sse < now {
            ms.stage = 2;
        }
    }

    /// UEquipmentModeSwitchMotion::OnLeave_Implementation rva=0x16659a0
    pub(crate) fn mode_switch_on_leave(&mut self, fi: usize, id: super::motion::MotionId) {
        if !self.m(fi, id).mode_switch().unwrap().b_has_finished_switch {
            self.finish_mode_switch(fi, id);
        }
    }

    /// UEquipmentModeSwitchMotion::FinishSwitch rva=0x165b4d0: bHasFinishedSwitch, then (authority, CheckCanEquipAlt)
    /// AMordhauCharacter::SwitchModeAndReAttach -> the mode swap
    fn finish_mode_switch(&mut self, fi: usize, id: super::motion::MotionId) {
        self.mm(fi, id).mode_switch_mut().unwrap().b_has_finished_switch = true;
        let f = &self.fighters[fi];
        if f.role != 3 || f.weapon.is_none() || !self.check_can_equip_alt(fi, f.weapon_equip.as_deref()) {
            return;
        }
        self.switch_mode_and_reattach(fi);
    }

    /// AMordhauCharacter::SwitchModeAndReAttach -> AMordhauWeapon::SwitchMode_Implementation rva=0x1640a00: the
    /// attacks, the equipment's Length / animation profile and the alternate flag swap (the former body of
    /// switch_mode; also the horse mount's direct switch, horse.rs)
    pub fn switch_mode_and_reattach(&mut self, fi: usize) {
        let Some(actor) = self.fighters[fi].right_actor else { return };
        let Some(a) = self.equipment.resolve_mut(actor) else { return };
        let Some(weapon) = a.weapon.as_ref() else { return };
        a.weapon = Some(Rc::new(weapon.switched()));
        a.equip = a.equip.as_ref().map(|e| Rc::new(e.switched()));
        a.alternate_mode = !a.alternate_mode;
        a.motion_bps = self.spec.profile_motions(a.equip.as_ref().map(|e| e.weapon_animation_profile.as_str()).unwrap_or("")).motions;
        self.project_equipment_hands(fi, false);
        let f = &mut self.fighters[fi];
        let s = format!("{} switch mode -> {}", f.name, if f.alternate_mode { "alternate" } else { "primary" });
        self.trace_event(&s);
    }
}

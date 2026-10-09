//! UMordhauMotion (base of every motion) + UIdleMotion + the virtual dispatch (godot/game/combat/combat_motion.gd).
//! A motion is the character's one current action. Time is the world clock in seconds (UWorld TimeSeconds, read at
//! world+0x598 by every motion function). Field names are the UE ones from extract/native/types/UMordhauMotion.h.

use super::attack::AttackMotion;
use super::blocked::BlockedMotion;
use super::feinted::FeintedMotion;
use super::parry::ParryMotion;
use super::react::{DisarmedMotion, FlinchMotion, StunMotion};
use super::world::World;
use crate::data::MotionDef;
use std::rc::Rc;

/// Index of a motion in its fighter's slab
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct MotionId(pub u32);

pub enum MotionKind {
    Idle,
    Attack(Box<AttackMotion>),
    Parry(Box<ParryMotion>),
    Feinted(Box<FeintedMotion>),
    Blocked(Box<BlockedMotion>),
    Flinch(Box<FlinchMotion>),
    Stun(Box<StunMotion>),
    Disarmed(Box<DisarmedMotion>),
    Climbing(Box<super::climb::ClimbingMotion>),
    /// UEnterVehicleMotion / ULeaveVehicleMotion (horse.rs)
    Vehicle(Box<super::horse::VehicleMotion>),
    /// URangedDrawMotion / URangedReleaseMotion / UReloadMotion / URangedCancelMotion (rangedmotion.rs)
    Ranged(Box<super::rangedmotion::RangedMotion>),
    /// UEquipmentModeSwitchMotion (modeswitch.rs, fp-anim r3)
    ModeSwitch(Box<super::modeswitch::ModeSwitchMotion>),
}

pub struct Motion {
    pub def: Rc<MotionDef>, // class defaults (native ctor + Blueprint CDO chain), shared read-only
    pub bp: String,         // Blueprint class path, "" for a native-only motion
    pub start_time: f64,    // +0x4c StartTime
    pub end_time: f64,      // +0x50 EndTime
    pub leave_time: f64,    // +0x54 LeaveTime
    pub expected_delay: f64, // +0x48 ExpectedDelay (network; 0 on one machine, authority)
    pub coming_from: Option<MotionId>, // +0x30 ComingFromMotion
    pub b_is_flinchable: bool, // +0x60
    pub b_can_attack: bool,    // +0x6d
    pub b_can_block: bool,     // +0x6e
    pub b_can_emote: bool,     // +0x6c
    pub b_blocks_regen: bool,  // +0x88 bBlocksRegen
    pub movement_restriction: i64, // +0x61 (read through UMordhauMotion::GetMovementRestriction rva=0x165cc20)
    /// +0x64 SpeedFactor / +0x68 BackpedalSpeedFactor: copied into the movement every actor tick
    /// (UMordhauMovementComponent::OnCharacterLODTick rva=0x14c9ec0)
    pub speed_factor: f64,
    pub backpedal_speed_factor: f64,
    pub k: MotionKind,
}

impl Motion {
    pub fn new(def: Rc<MotionDef>, k: MotionKind) -> Motion {
        let b = &def.base;
        Motion {
            bp: b.path.clone(),
            start_time: 0.0,
            end_time: 0.0,
            leave_time: 0.0,
            expected_delay: 0.0,
            coming_from: None,
            b_is_flinchable: b.b_is_flinchable,
            b_can_attack: b.b_can_attack,
            b_can_block: b.b_can_block,
            b_can_emote: b.b_can_emote,
            b_blocks_regen: b.b_blocks_regen,
            movement_restriction: b.movement_restriction,
            speed_factor: b.speed_factor,
            backpedal_speed_factor: b.backpedal_speed_factor,
            k,
            def,
        }
    }
    pub fn kind(&self) -> &'static str {
        match &self.k {
            MotionKind::Idle => "Idle",
            MotionKind::Attack(_) => "Attack",
            MotionKind::Parry(_) => "Parry",
            MotionKind::Feinted(_) => "Feinted",
            MotionKind::Blocked(_) => "Blocked",
            MotionKind::Flinch(_) => "Flinch",
            MotionKind::Stun(_) => "Stun",
            MotionKind::Disarmed(_) => "Disarmed",
            MotionKind::Climbing(_) => "Climbing",
            MotionKind::Vehicle(v) => if v.leaving { "LeaveVehicle" } else { "EnterVehicle" },
            MotionKind::ModeSwitch(_) => "EquipmentModeSwitch",
            MotionKind::Ranged(r) => match r.stage {
                super::rangedmotion::RangedStage::Draw => "RangedDraw",
                super::rangedmotion::RangedStage::Release => "RangedRelease",
                super::rangedmotion::RangedStage::Reload => "Reload",
                super::rangedmotion::RangedStage::Cancel => "RangedCancel",
            },
        }
    }
    pub fn attack(&self) -> Option<&AttackMotion> {
        if let MotionKind::Attack(a) = &self.k { Some(a) } else { None }
    }
    pub fn attack_mut(&mut self) -> Option<&mut AttackMotion> {
        if let MotionKind::Attack(a) = &mut self.k { Some(a) } else { None }
    }
    pub fn parry(&self) -> Option<&ParryMotion> {
        if let MotionKind::Parry(a) = &self.k { Some(a) } else { None }
    }
    pub fn parry_mut(&mut self) -> Option<&mut ParryMotion> {
        if let MotionKind::Parry(a) = &mut self.k { Some(a) } else { None }
    }
    pub fn feinted(&self) -> Option<&FeintedMotion> {
        if let MotionKind::Feinted(a) = &self.k { Some(a) } else { None }
    }
    pub fn feinted_mut(&mut self) -> Option<&mut FeintedMotion> {
        if let MotionKind::Feinted(a) = &mut self.k { Some(a) } else { None }
    }
    pub fn blocked(&self) -> Option<&BlockedMotion> {
        if let MotionKind::Blocked(a) = &self.k { Some(a) } else { None }
    }
    pub fn blocked_mut(&mut self) -> Option<&mut BlockedMotion> {
        if let MotionKind::Blocked(a) = &mut self.k { Some(a) } else { None }
    }
    pub fn mode_switch(&self) -> Option<&super::modeswitch::ModeSwitchMotion> {
        if let MotionKind::ModeSwitch(a) = &self.k { Some(a) } else { None }
    }
    pub fn mode_switch_mut(&mut self) -> Option<&mut super::modeswitch::ModeSwitchMotion> {
        if let MotionKind::ModeSwitch(a) = &mut self.k { Some(a) } else { None }
    }
    pub fn is_attack(&self) -> bool {
        matches!(self.k, MotionKind::Attack(_))
    }
    pub fn is_parry(&self) -> bool {
        matches!(self.k, MotionKind::Parry(_))
    }
    pub fn is_flinch(&self) -> bool {
        matches!(self.k, MotionKind::Flinch(_))
    }
}

/// The motion "virtual table": each method is the UMordhauMotion virtual, dispatched on the motion's class.
impl World {
    // ---- accessors ----
    pub fn m(&self, fi: usize, id: MotionId) -> &Motion {
        self.fighters[fi].motions[id.0 as usize].as_ref().expect("freed motion")
    }
    pub fn mm(&mut self, fi: usize, id: MotionId) -> &mut Motion {
        self.fighters[fi].motions[id.0 as usize].as_mut().expect("freed motion")
    }
    pub fn att(&self, fi: usize, id: MotionId) -> &AttackMotion {
        self.m(fi, id).attack().expect("not an attack motion")
    }
    pub fn att_mut(&mut self, fi: usize, id: MotionId) -> &mut AttackMotion {
        self.mm(fi, id).attack_mut().expect("not an attack motion")
    }
    pub fn is_current(&self, fi: usize, id: MotionId) -> bool {
        self.fighters[fi].motion == Some(id)
    }

    // ---- virtuals ----
    pub fn motion_on_begin(&mut self, fi: usize, id: MotionId) {
        match self.m(fi, id).k {
            MotionKind::Idle => self.idle_on_begin(fi, id),
            MotionKind::Attack(_) => {
                self.attack_on_begin(fi, id);
                self.attack_turn_caps_on_begin(fi, id);
            }
            MotionKind::Parry(_) => self.parry_on_begin(fi, id),
            MotionKind::Feinted(_) => self.feinted_on_begin(fi, id),
            MotionKind::Blocked(_) => self.blocked_on_begin(fi, id),
            MotionKind::Flinch(_) => self.flinch_on_begin(fi, id),
            MotionKind::Stun(_) => self.stun_on_begin(fi, id),
            MotionKind::Disarmed(_) => self.disarmed_on_begin(fi, id),
            MotionKind::Climbing(_) => self.climbing_on_begin(fi, id),
            MotionKind::Vehicle(_) => self.vehicle_on_begin(fi, id),
            MotionKind::Ranged(_) => self.ranged_on_begin(fi, id),
            MotionKind::ModeSwitch(_) => self.mode_switch_on_begin(fi, id),
        }
    }

    pub fn motion_on_tick(&mut self, fi: usize, id: MotionId, dt: f64) {
        match self.m(fi, id).k {
            MotionKind::Idle => self.idle_on_tick(fi, id),
            MotionKind::Attack(_) => self.attack_on_tick(fi, id),
            MotionKind::Parry(_) => self.parry_on_tick(fi, id, dt),
            MotionKind::Feinted(_) => self.feinted_on_tick(fi, id),
            MotionKind::Blocked(_) => self.blocked_on_tick(fi, id),
            MotionKind::Flinch(_) => self.flinch_on_tick(fi, id),
            MotionKind::Stun(_) => self.stun_on_tick(fi, id),
            MotionKind::Ranged(_) => self.ranged_on_tick(fi, id, dt),
            MotionKind::ModeSwitch(_) => self.mode_switch_on_tick(fi, id),
            MotionKind::Disarmed(_) | MotionKind::Climbing(_) | MotionKind::Vehicle(_) => {}
        }
    }

    pub fn motion_on_leave(&mut self, fi: usize, id: MotionId, _interrupted: bool) {
        match self.m(fi, id).k {
            MotionKind::Attack(_) => self.attack_on_leave(fi, id),
            MotionKind::Parry(_) => self.parry_on_leave(fi, id),
            MotionKind::Blocked(_) => self.blocked_on_leave(fi, id),
            MotionKind::Ranged(_) => self.ranged_on_leave(fi, id),
            MotionKind::Stun(_) => self.stun_on_leave(fi),
            MotionKind::ModeSwitch(_) => self.mode_switch_on_leave(fi, id),
            _ => {}
        }
    }

    pub fn motion_on_dynamic_param_changed(&mut self, fi: usize, id: MotionId, old: i64, nv: i64) {
        match self.m(fi, id).k {
            MotionKind::Attack(_) => self.attack_on_dynamic_param_changed(fi, id, nv),
            MotionKind::Parry(_) => self.parry_on_dynamic_param_changed(fi, id, old, nv),
            _ => {}
        }
    }

    pub fn motion_on_ended(&mut self, fi: usize, id: MotionId) {
        match self.m(fi, id).k {
            MotionKind::Attack(_) => self.attack_on_ended(fi, id),
            MotionKind::Feinted(_) => self.feinted_on_ended(fi, id),
            MotionKind::Blocked(_) => self.blocked_on_ended(fi, id),
            _ => self.base_on_ended(fi, id),
        }
    }

    pub fn motion_process_attack(&mut self, fi: usize, id: MotionId, mv: i64, angle: f64) -> bool {
        match self.m(fi, id).k {
            MotionKind::Attack(_) => self.attack_process_attack(fi, id, mv, angle),
            MotionKind::Parry(_) => self.parry_process_attack(fi, id, mv, angle),
            MotionKind::Feinted(_) => self.feinted_process_attack(fi, id, mv, angle),
            MotionKind::Blocked(_) => self.blocked_process_attack(fi, id, mv, angle),
            _ => self.base_process_attack(fi, id, mv, angle),
        }
    }

    pub fn motion_process_block(&mut self, fi: usize, id: MotionId, bt: i64) -> bool {
        match self.m(fi, id).k {
            MotionKind::Attack(_) => self.attack_process_block(fi, id, bt),
            MotionKind::Parry(_) => self.parry_process_block(fi, id, bt),
            MotionKind::Feinted(_) => self.feinted_process_block(fi, id, bt),
            _ => self.base_process_block(fi, id, bt),
        }
    }

    pub fn motion_process_feint(&mut self, fi: usize, id: MotionId) -> bool {
        match self.m(fi, id).k {
            MotionKind::Attack(_) => self.attack_process_feint(fi, id),
            MotionKind::Parry(_) => self.parry_process_feint(fi, id),
            MotionKind::Feinted(_) => {
                // UFeintedMotion::ProcessFeint_Implementation rva=0x166bd10: drops the queued attack
                self.mm(fi, id).feinted_mut().unwrap().b_has_queued_move = false;
                false
            }
            // UMordhauMotion::ProcessFeint_Implementation rva=0x7bf520 (shared "return 0" stub)
            _ => false,
        }
    }

    /// UMordhauMotion::CanInitiateMotion_Implementation rva=0x7bf520 (shared stub `xor al, al; ret`) unless the class
    /// overrides it: UIdleMotion / UFeintedMotion rva=0x7bf3e0 (`mov al, 1; ret`), UParryMotion rva=0x164ea00,
    /// UFlinchMotion rva=0x164e6e0. new_kind = the requested motion class's kind name.
    pub fn motion_can_initiate(&self, fi: usize, id: MotionId, new_kind: &str) -> bool {
        let m = self.m(fi, id);
        match &m.k {
            MotionKind::Idle | MotionKind::Feinted(_) => true,
            MotionKind::Parry(p) => {
                // only a parry that blocked something (TotalBlocks +0x538 != 0) and is not a shield wall (+0x46a)
                if p.total_blocks != 0 {
                    !p.b_is_shield_wall
                } else {
                    false
                }
            }
            // UFlinchMotion: only UInteractWithMotion, UEquipmentSwitchMotion, UEquipmentModeSwitchMotion
            MotionKind::Flinch(_) => matches!(new_kind, "InteractWith" | "EquipmentSwitch" | "EquipmentModeSwitch"),
            _ => false,
        }
    }

    /// UMordhauMotion::GetMovementRestriction rva=0x165cc20 returns the field; UAttackMotion / UKickMotion /
    /// UParryMotion override it.
    pub fn motion_movement_restriction(&self, fi: usize, id: MotionId) -> i64 {
        match self.m(fi, id).k {
            MotionKind::Attack(_) => self.attack_movement_restriction(fi, id),
            MotionKind::Parry(_) => self.parry_movement_restriction(fi, id),
            _ => self.m(fi, id).movement_restriction,
        }
    }

    /// UMordhauMotion::GetAttackCompensationStartTime rva=0x165c630: !bCanAttack -> EndTime, else 0;
    /// UBlockedMotion overrides it (rva=0x165c460).
    pub fn motion_attack_compensation_start_time(&self, fi: usize, id: MotionId, mv: i64) -> f64 {
        let m = self.m(fi, id);
        if m.blocked().is_some() {
            return self.blocked_attack_compensation_start_time(fi, id, mv);
        }
        if !m.b_can_attack { m.end_time } else { 0.0 }
    }

    // ---- base bodies ----
    /// from UMordhauMotion::Tick rva=0x166ee80: OnTick, then OnEnded once EndTime <= now
    pub fn motion_tick(&mut self, fi: usize, id: MotionId, dt: f64) {
        self.motion_on_tick(fi, id, dt);
        // Exe (UMordhauMotion::Tick rva=0x166ee80, decomp: `EndTime < TimeSeconds || EndTime == TimeSeconds` ->
        // OnEnded, no current-motion test; byte-matched src/Mordhau/Private/Motions/MordhauMotion.cpp): OnEnded runs
        // even when OnTick already switched the motion (UAttackMotion / UBlockedMotion::OnEnded then still fire their
        // queued move). The reference adds `sys.motion == self` (exe.rs difference 3).
        let current_ok = self.exe() || self.is_current(fi, id);
        if current_ok && self.m(fi, id).end_time <= self.now {
            self.motion_on_ended(fi, id);
        }
    }

    /// from UMordhauMotion::OnEnded_Implementation rva=0x16638a0: if still current, ChangeMotion(UIdleMotion)
    pub fn base_on_ended(&mut self, fi: usize, id: MotionId) {
        if self.is_current(fi, id) {
            self.change_to_idle(fi);
        }
    }

    /// from UMordhauMotion::ProcessAttack_Implementation rva=0x166b7c0
    pub fn base_process_attack(&mut self, fi: usize, id: MotionId, mv: i64, angle: f64) -> bool {
        if !self.m(fi, id).b_can_attack {
            return false;
        }
        if self.now < self.fighters[fi].next_attack_time {
            // MotionSystem+0xb4 NextAttackTime
            return false;
        }
        self.assign_net_attack_motion(fi, super::enums::at::REGULAR, mv, angle);
        !self.is_current(fi, id)
    }

    /// from UMordhauMotion::ProcessBlock_Implementation rva=0x166bb20: FNetMotion{MotionType 2 (Parry), Param0 type}
    pub fn base_process_block(&mut self, fi: usize, id: MotionId, bt: i64) -> bool {
        if !self.m(fi, id).b_can_block {
            return false;
        }
        self.assign_net_parry(fi, bt);
        true
    }

    // ---- UIdleMotion ----
    /// from UIdleMotion::OnBegin_Implementation rva=0x1660d30: EndTime = FLT_MAX (immediate 0x7f7fffff),
    /// bCanEmote = 1, bBlocksRegen = 0
    fn idle_on_begin(&mut self, fi: usize, id: MotionId) {
        let exe = self.exe();
        let m = self.mm(fi, id);
        // The exe stores FLT_MAX (0x7f7fffff) = f64 bits 0x47efffffe0000000. The GDScript reference writes the literal
        // 3.4028234663852886e38, which Godot's parser turns into 0x47efffffe0000001 (1 ulp above; a reference
        // artifact reported to the combat owner). Mirrored here so golden parity stays bit-exact; no rule compares
        // against it beyond `EndTime <= now`, which neither value reaches.
        m.end_time = if exe { f32::MAX as f64 } else { f64::from_bits(0x47efffffe0000001) }; // exe: 0x7f7fffff
        m.b_can_emote = true;
        m.b_blocks_regen = false;
    }

    /// from UIdleMotion::OnTick_Implementation rva=0x16684c0: ComingFromMotion cleared idle_coming_from_timeout
    /// (0.5 s, .rdata 0x143fe4e04) after start
    fn idle_on_tick(&mut self, fi: usize, id: MotionId) {
        let timeout = self.spec.constants.idle_coming_from_timeout;
        let now = self.now;
        let q = self.qf();
        let m = self.mm(fi, id);
        if q(m.start_time + timeout) < now {
            m.coming_from = None;
        }
    }
}

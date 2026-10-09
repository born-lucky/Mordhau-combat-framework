//! Turn caps (rust-combat r4, asked by rust-parity r6): the rate limit an attack puts on the character's turning.
//!
//! - UAttackMotion::ModifyAttackInfo copies AttackInfo.TurnCaps (FVector2D +0x34) / TurnCapCurve (+0x40) (decomp
//!   UAttackMotion.cpp 1442-1443); OnBegin_Implementation rva=0x162eda0 calls SetTurnCaps(TurnCaps.X, TurnCaps.Y)
//!   (vcall +0x9e8, decomp 1985); OnTick_Implementation rva=0x16328c0 (2480-2510): TurnValueSum += the actor yaw's
//!   change since PreviousTurnValue (wrapped to +-180), and with a TurnCapCurve SetTurnCaps(Curve(|sum|) * TurnCaps)
//!   (every AttackInfo in the spec matrix has no TurnCapCurve: not modelled); EnterRecovery rva=0x161b7b0 re-applies
//!   TurnCaps when LagInduction >= 0.03 on an autonomous proxy (decomp 3207-3222; offline LagInduction is 0);
//!   OnLeave_Implementation rva=0x16323e0 SetTurnCaps(-1, -1) (decomp 5922-5924).
//! - AMordhauCharacter::SetTurnCaps rva=0x156c4c0: both values (unless -1) times the current motion's TurncapModifier
//!   (UMordhauMotion ctor 1.0, UMordhauMotion.cpp 211; only perks change it), then AAdvancedCharacter::SetTurnCaps
//!   rva=0x14a15f0: Turn != -1 and (TurnRateCap == -1 or Turn < TurnRateCap) -> TurnRateCap = Turn,
//!   TurnCapRemaining = min(TurnCapRemaining, Turn * 0.0333); TurnRateCapTarget = Turn; the same for look-up.
//! - AAdvancedCharacter::LODTick rva=0x14887b0 (decomp 524-576), before the motion tick: TurnRateCap != -1 ->
//!   Target -1: Cap += dt * 1500; else Cap < Target: FInterpConstantTo(Cap, Target, dt, 4500); Remaining =
//!   clamp(Cap * dt + Remaining, 0, Cap * 0.0333); Target -1 and Cap > 1500 -> Cap = -1. Look-up: 1050 / 3150.
//! - AAdvancedCharacter::Turn rva=0x14a8a90: a turn input Value (degrees) with TurnRateCap != -1 is clamped to
//!   +-TurnCapRemaining and TurnCapRemaining -= |clamped|.
//! UNCONFIRMED: where the controller's turn axis runs relative to the attack request inside one frame (the Sim turns
//! before the actor tick, with the caps of the previous tick). The look-up clamp is AAdvancedCharacter::LookUp
//! rva=0x1489930 (cap_look_up).

use super::World;

/// AAdvancedCharacter turn-cap state (+TurnRateCap, TurnCapRemaining, TurnRateCapTarget and the look-up trio)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TurnCaps {
    pub turn_rate_cap: f64,
    pub turn_cap_remaining: f64,
    pub turn_rate_cap_target: f64,
    pub look_up_rate_cap: f64,
    pub look_up_cap_remaining: f64,
    pub look_up_rate_cap_target: f64,
}

impl Default for TurnCaps {
    /// AAdvancedCharacter ctor (decomp AAdvancedCharacter.cpp 1503: TurnRateCap = -1; the rest zero / -1 likewise)
    fn default() -> Self {
        TurnCaps { turn_rate_cap: -1.0, turn_cap_remaining: 0.0, turn_rate_cap_target: -1.0, look_up_rate_cap: -1.0, look_up_cap_remaining: 0.0, look_up_rate_cap_target: -1.0 }
    }
}

/// FMath::FInterpConstantTo (engine source, UNCONFIRMED as compiled)
fn finterp_constant_to(q: fn(f64) -> f64, cur: f64, target: f64, dt: f64, speed: f64) -> f64 {
    let d = q(target - cur);
    if q(d * d) < 1e-8 {
        return target;
    }
    let step = q(speed * dt);
    q(cur + d.clamp(-step, step))
}

impl World {
    /// AMordhauCharacter::SetTurnCaps rva=0x156c4c0 -> AAdvancedCharacter::SetTurnCaps rva=0x14a15f0
    pub fn set_turn_caps(&mut self, fi: usize, turn: f64, look_up: f64) {
        let q = self.qf();
        let c = &mut self.fighters[fi].turn_caps;
        // TurncapModifier = 1 (no perks)
        if turn != -1.0 && (c.turn_rate_cap == -1.0 || turn < c.turn_rate_cap) {
            c.turn_rate_cap = turn;
            let r = q(turn * q(0.0333));
            c.turn_cap_remaining = if c.turn_cap_remaining <= r { c.turn_cap_remaining } else { r };
        }
        c.turn_rate_cap_target = turn;
        if look_up != -1.0 && (c.look_up_rate_cap == -1.0 || look_up < c.look_up_rate_cap) {
            c.look_up_rate_cap = look_up;
            let r = q(look_up * q(0.0333));
            c.look_up_cap_remaining = if c.look_up_cap_remaining <= r { c.look_up_cap_remaining } else { r };
        }
        c.look_up_rate_cap_target = look_up;
    }

    /// UAttackMotion::OnBegin_Implementation rva=0x162eda0 (decomp 1985, after PlayAnim): SetTurnCaps(AttackInfo.TurnCaps)
    /// with the motion's AttackInfo (ModifyAttackInfo copies TurnCaps unchanged, decomp 1442)
    pub(crate) fn attack_turn_caps_on_begin(&mut self, fi: usize, id: super::motion::MotionId) {
        if !self.is_current(fi, id) {
            return;
        }
        let tc = self.att(fi, id).ai.turn_caps.clone();
        if tc.len() == 2 {
            self.set_turn_caps(fi, tc[0], tc[1]);
        }
    }

    /// the LODTick part (module docs)
    pub(crate) fn tick_turn_caps(&mut self, fi: usize, dt: f64) {
        let q = self.qf();
        let c = &mut self.fighters[fi].turn_caps;
        let tick = |cap: &mut f64, rem: &mut f64, target: f64, up: f64, interp: f64| {
            if *cap == -1.0 {
                return;
            }
            if target == -1.0 {
                *cap = q(q(dt * up) + *cap);
            } else if *cap < target {
                *cap = finterp_constant_to(q, *cap, target, dt, interp);
            }
            let mut r = q(q(*cap * dt) + *rem);
            let hi = q(*cap * q(0.0333));
            if r < 0.0 {
                r = 0.0;
            } else if hi <= r {
                r = hi;
            }
            *rem = r;
            if target == -1.0 && up < *cap {
                *cap = -1.0;
            }
        };
        let (mut a, mut b) = (c.turn_rate_cap, c.turn_cap_remaining);
        tick(&mut a, &mut b, c.turn_rate_cap_target, 1500.0, 4500.0);
        c.turn_rate_cap = a;
        c.turn_cap_remaining = b;
        let (mut a, mut b) = (c.look_up_rate_cap, c.look_up_cap_remaining);
        tick(&mut a, &mut b, c.look_up_rate_cap_target, 1050.0, 3150.0);
        c.look_up_rate_cap = a;
        c.look_up_cap_remaining = b;
    }

    /// AAdvancedCharacter::Turn rva=0x14a8a90: the turn actually applied for a requested yaw change (degrees)
    pub fn cap_turn(&mut self, fi: usize, value: f64) -> f64 {
        let q = self.qf();
        let c = &mut self.fighters[fi].turn_caps;
        if value == 0.0 || c.turn_rate_cap == -1.0 {
            return value;
        }
        let r = c.turn_cap_remaining;
        let v = value.clamp(-r, r);
        c.turn_cap_remaining = q(r - v.abs()).max(0.0);
        v
    }

    /// AAdvancedCharacter::LookUp rva=0x1489930 (decomp AAdvancedCharacter.cpp 3866-3884, fidelity-audit): with
    /// LookUpRateCap != -1 and !bIsAbsolute the value is clamped to +-LookUpCapRemaining, which drops by |v| (floored at 0);
    /// the host then clamps LookUpValue to (-LookDownLimit, LookUpLimit)
    pub fn cap_look_up(&mut self, fi: usize, value: f64) -> f64 {
        let q = self.qf();
        let c = &mut self.fighters[fi].turn_caps;
        if value == 0.0 || c.look_up_rate_cap == -1.0 {
            return value;
        }
        let r = c.look_up_cap_remaining;
        let v = value.clamp(-r, r);
        c.look_up_cap_remaining = q(r - v.abs()).max(0.0);
        v
    }
}

//! The character-level pieces around the movement model:
//!  - `CharacterInput`: the model half of MordhauCharacter.step (godot/game/character/mordhau_character.gd
//!    `step_model`): control yaw -> facing, axes -> wish direction, button edges -> AMordhauCharacter::SprintPressed /
//!    SprintReleased / CrouchPressed / CrouchReleased, MoveForward's toggle-sprint stop, the LODTick crouch toggle,
//!    jump, then MordhauMovement::tick. The body (capsule, collision, floor) is the host's: it moves by the returned
//!    delta and calls MordhauMovement::set_on_floor.
//!  - `apply_movement_events`: Fighter.apply_movement_events (godot/game/actor/fighter.gd), the once-per-frame link to
//!    the combat side (motion restriction in; jump stamina, fall damage out), behind the `CombatSide` trait so the sim
//!    facade (mordhau-core) plugs its MotionSystem in.

use crate::compat::movement::{Mode, MordhauMovement};
use crate::ue::{basis_axis_angle, basis_col, deg_to_rad, FVector, V3Ext};

/// Per-pawn input state of MordhauCharacter: control yaw (degrees, `yaw`) and last frame's Sprint / Crouch buttons.
#[derive(Clone, Debug, Default)]
pub struct CharacterInput {
    pub yaw: f64,
    pub sprint_held: bool,
    pub crouch_held: bool,
}

/// What one step returns to the host.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StepOut {
    /// position delta in cm (Godot axes); the body moves by it (MordhauCharacter.step: velocity = delta * 0.01 / dt)
    pub delta: FVector,
    /// actor forward (Godot -Z rotated by the control yaw)
    pub facing: FVector,
    /// Some(true) / Some(false) = Crouch() / UnCrouch() applied this step (the host resizes the capsule:
    /// CrouchedHalfHeight vs the AMordhauCharacter ctor's 96), None = no change
    pub crouch: Option<bool>,
}

impl CharacterInput {
    /// MordhauCharacter.step_model. fwd/right: -1..1 axes ("Move Forward" / "Move Right", DefaultInput.ini).
    #[allow(clippy::too_many_arguments)]
    pub fn step(
        &mut self,
        m: &mut MordhauMovement,
        dt: f64,
        fwd: f64,
        right: f64,
        jump: bool,
        sprint: bool,
        crouch: bool,
    ) -> StepOut {
        let b = basis_axis_angle(FVector::new(0.0, 1.0, 0.0), deg_to_rad(self.yaw));
        let facing = basis_col(&b, 2).neg();
        let wish = (facing.scale(fwd) + basis_col(&b, 0).scale(right)).limit_length(1.0);
        // button edges -> AMordhauCharacter::SprintPressed / SprintReleased / CrouchPressed / CrouchReleased
        if sprint != self.sprint_held {
            if sprint {
                m.sprint_pressed();
            } else {
                m.sprint_released();
            }
            self.sprint_held = sprint;
        }
        m.move_forward_axis(fwd);
        if crouch != self.crouch_held {
            if crouch {
                m.crouch_pressed();
            } else {
                m.crouch_released();
            }
            self.crouch_held = crouch;
        }
        // AMordhauCharacter::LODTick crouch cooldown -> Crouch() / UnCrouch(), applied while walking
        let mut c = m.update_crouch();
        match c {
            Some(on) if m.mode == Mode::Walking => m.crouched = on,
            _ => c = None,
        }
        if jump {
            m.do_jump();
        }
        let delta = m.tick(dt, wish, facing);
        StepOut { delta, facing, crouch: c }
    }

    /// MordhauCharacter.reset_pawn's input part: a respawned pawn starts with no buttons held, look pitch 0 (the host's)
    pub fn reset(&mut self) {
        self.sprint_held = false;
        self.crouch_held = false;
    }
}

/// The combat side a fighter's movement talks to once per frame (the reference's MotionSystem calls in
/// Fighter.apply_movement_events). The sim facade implements it on its fighter.
pub trait CombatSide {
    /// UMordhauMotion::GetMovementRestriction of the current motion (0 without one)
    fn motion_movement_restriction(&self) -> i64;
    /// AAdvancedCharacter bIsDead as the combat side keeps it
    fn dead(&self) -> bool;
    /// AMordhauCharacter +0xe7c JumpStaminaCost (records::Character::jump_stamina_cost)
    fn jump_stamina_cost(&self) -> f64;
    /// UStaminaStatComponent OffsetStamina via MotionSystem.offset_stamina
    fn offset_stamina(&mut self, amount: i64);
    /// the stamina component's StopRegeneration(delay) (MotionSystem.stop_stamina_regen)
    fn stop_stamina_regen(&mut self, delay: f64);
    /// TakeDamage(amount, EMordhauDamageType::Fall) with no source (MotionSystem.take_damage(d, null, DAMAGE_FALL))
    fn take_fall_damage(&mut self, amount: f64);
}

/// Fighter.apply_movement_events (fighter.gd), once per frame before the combat step (FighterSim.frame):
///  - the current motion's EMovementRestriction (UMordhauMotion::GetMovementRestriction, combat side) goes into the
///    movement, which folds it into AMordhauCharacter::GetMovementRestriction rva=0x1540f50 (equipment restriction
///    not ported: UNCONFIRMED for equipment that sets one)
///  - each jump since last frame: AMordhauCharacter::OnJumped_Implementation rva=0x1554610 ->
///    OffsetStamina(-(int)JumpStaminaCost) then the stamina component's StopRegeneration(0)
///  - each CheckFallDamage amount: TakeDamage(amount, Fall) (UAdvancedCharacterMovement::CheckFallDamage rva=0x145a230);
///    the causer is the character itself, passed as no source (the IsFriendly(self, self) spawn-window scaling of
///    AMordhauCharacter::TakeDamage is UNCONFIRMED for self-damage and not applied)
///  - SetIsRagdollFalling changes would assign the RagdollFalling net motion (MotionType 0x1a,
///    AMordhauCharacter::SetIsRagdollFalling rva=0x1568ad0) on the combat side: not ported in the reference either
///    (no URagdollFallingMotion), the changes are dropped. UNCONFIRMED: the combat state during a ragdoll.
/// With no combat side (`None`) the events are only drained.
pub fn apply_movement_events<C: CombatSide + ?Sized>(m: &mut MordhauMovement, sys: Option<&mut C>) {
    let Some(sys) = sys else {
        m.jumps = 0;
        m.fall_damage.clear();
        m.ragdoll_changes.clear();
        return;
    };
    m.motion_restriction = sys.motion_movement_restriction();
    m.dead = sys.dead();
    for _ in 0..m.jumps {
        sys.offset_stamina(-(sys.jump_stamina_cost() as i64)); // GDScript -int(x): truncation toward zero
        sys.stop_stamina_regen(0.0);
    }
    m.jumps = 0;
    for d in m.fall_damage.drain(..) {
        sys.take_fall_damage(d);
    }
    m.ragdoll_changes.clear();
}

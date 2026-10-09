//! AMordhauGameMode's spawn queue (godot/game/mode/spawn_queue.gd), shared by every mode.
//!
//! AMordhauGameMode::RestartPlayer rva=0x15a6d50: append to SpawnQueue (+0x3b0) unless already queued or currently
//! spawning (CurrentlySpawningController +0x3c0).
//! AMordhauGameMode::Tick rva=0x15ab000, one step per tick (the do-while repeats only in the editor: +0x3dc is set to 1
//! when !IsEditor()); the step counter is +0x3d0:
//!   step 0: pop the queue front (skip destroyed), CurrentlySpawningController = it, PrepareControllerForRespawn
//!           (0x15a4190), spawn the pawn (func_0x1435e27c0 = SpawnDefaultPawnFor via ChoosePlayerStart) -> step 1
//!   step 1: controller still valid -> vtable +0x790 = AMordhauGameMode::RestartPlayerAtPlayerStart (0x15a6e60, resolved
//!           from AMordhauGameMode::`vftable' 0x14434c070 + 0x790): possess the new pawn -> step 2, else reset
//!   step 2: FinalizeSpawnedCharacter (0x15938f0) on the controller's pawn -> reset
//! The owner (GameMode::spawn_queue_step) supplies validity, the spawn itself and the events.

use crate::game_mode::CtrlId;

#[derive(Clone, Debug, Default)]
pub struct SpawnQueue {
    pub queue: Vec<CtrlId>,
    pub spawning: Option<CtrlId>,
    pub step: i32,
}

/// what one queue step asks its owner to do
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum SpawnStep {
    /// step 0: spawn the pawn for this controller (SpawnDefaultPawnFor -> ChoosePlayerStart)
    Spawn(CtrlId),
    /// step 1: possess (the owner sets has_pawn / alive and emits "possess")
    Possess(CtrlId),
    /// step 2: FinalizeSpawnedCharacter
    Finalize(CtrlId),
    None,
}

impl SpawnQueue {
    pub fn restart_player(&mut self, c: CtrlId) {
        if self.queue.contains(&c) || self.spawning == Some(c) {
            return;
        }
        self.queue.push(c);
    }

    pub fn is_queued(&self, c: CtrlId) -> bool {
        self.queue.contains(&c) || self.spawning == Some(c)
    }

    /// one Tick step; `valid(c)`: the controller is still logged in
    pub fn tick(&mut self, valid: impl Fn(CtrlId) -> bool) -> SpawnStep {
        match self.step {
            0 => {
                while !self.queue.is_empty() {
                    let c = self.queue.remove(0);
                    if !valid(c) {
                        continue;
                    }
                    self.spawning = Some(c);
                    self.step = 1;
                    return SpawnStep::Spawn(c);
                }
                SpawnStep::None
            }
            1 => match self.spawning {
                Some(c) if valid(c) => {
                    self.step = 2;
                    SpawnStep::Possess(c)
                }
                _ => {
                    self.spawning = None;
                    self.step = 0;
                    SpawnStep::None
                }
            },
            2 => {
                let s = self.spawning.take();
                self.step = 0;
                match s {
                    Some(c) if valid(c) => SpawnStep::Finalize(c),
                    _ => SpawnStep::None,
                }
            }
            _ => SpawnStep::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn three_steps_one_per_tick() {
        let mut q = SpawnQueue::default();
        q.restart_player(3);
        q.restart_player(3);
        q.restart_player(5);
        assert_eq!(q.queue, vec![3, 5]);
        assert_eq!(q.tick(|_| true), SpawnStep::Spawn(3));
        assert!(q.is_queued(3));
        q.restart_player(3); // currently spawning: not re-queued
        assert_eq!(q.queue, vec![5]);
        assert_eq!(q.tick(|_| true), SpawnStep::Possess(3));
        assert_eq!(q.tick(|_| true), SpawnStep::Finalize(3));
        assert_eq!(q.tick(|c| c != 5), SpawnStep::None); // invalid controllers are skipped
        assert!(q.queue.is_empty());
    }
}

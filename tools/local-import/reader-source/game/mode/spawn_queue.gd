# spawn_queue.gd - AMordhauGameMode's spawn queue, shared by every mode port (DuelMode, FfaMode). Engine-agnostic:
# controllers are any objects with `has_pawn` / `alive` fields; the owner passes `valid` (still logged in) and
# `spawn` (spawn the pawn: returns the event extras, e.g. the chosen start) callables and receives events.
#
# AMordhauGameMode::RestartPlayer rva=0x15a6d50: append to SpawnQueue (+0x3b0) unless already queued or currently
# spawning (CurrentlySpawningController +0x3c0).
# AMordhauGameMode::Tick rva=0x15ab000, one step per tick (the do-while repeats only in the editor: +0x3dc is set to 1
# when !IsEditor()); the step counter is +0x3d0:
#   step 0: pop the queue front (skip destroyed), CurrentlySpawningController = it, PrepareControllerForRespawn
#           (0x15a4190), spawn the pawn (func_0x1435e27c0 = SpawnDefaultPawnFor via ChoosePlayerStart) -> step 1
#   step 1: controller still valid -> vtable +0x790 = AMordhauGameMode::RestartPlayerAtPlayerStart (0x15a6e60, resolved
#           from AMordhauGameMode::`vftable' 0x14434c070 + 0x790): possess the new pawn -> step 2, else reset
#   step 2: FinalizeSpawnedCharacter (0x15938f0) on the controller's pawn -> reset
class_name SpawnQueue
extends RefCounted

var queue: Array = []
var spawning = null
var step := 0

func restart_player(c) -> void:
	if queue.has(c) or spawning == c:
		return
	queue.append(c)

func is_queued(c) -> bool:
	return queue.has(c) or spawning == c

# one Tick step; emit(kind, extra) receives "spawn_pawn" (extra from spawn.call(c)), "possess", "finalize_spawn"
func tick(valid: Callable, spawn: Callable, emit: Callable) -> void:
	if step == 0:
		while not queue.is_empty():
			var c = queue.pop_front()
			if not valid.call(c):
				continue
			spawning = c
			var extra: Dictionary = spawn.call(c)
			emit.call("spawn_pawn", c, extra)
			step = 1
			return
	elif step == 1:
		if spawning != null and valid.call(spawning):
			spawning.has_pawn = true
			spawning.alive = true		# bIsAlive with the possessed pawn (setter not ported - UNCONFIRMED)
			emit.call("possess", spawning, {})
			step = 2
		else:
			spawning = null
			step = 0
	elif step == 2:
		if spawning != null and valid.call(spawning):
			emit.call("finalize_spawn", spawning, {})
		spawning = null
		step = 0

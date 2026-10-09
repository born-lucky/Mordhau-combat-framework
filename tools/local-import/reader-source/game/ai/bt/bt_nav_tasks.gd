# bt_nav_tasks.gd - the deathmatch tree's Blueprint movement tasks, ported from their bytecode (scripts/kismet;
# statement indices in brackets; literals: ModeKismet "ai_*"). Navigation queries go through the controller's
# random_reachable_point (UNavigationSystemV1::GetRandomReachablePointInRadius stand-in, MordhauBotController); the
# team navigation filter (GetTeamFilterClass) is not modelled (UNCONFIRMED).
#   FindRandomLocation  BTTask_FindRandomLocation_C (Mordhau/Content/Mordhau/AI/Tasks/BTTask_FindRandomLocation)
#   FindUnstuckSpot     BTTask_FindUnstuckSpot_C (Mordhau/Content/Mordhau/AI/Tasks/BTTask_FindUnstuckSpot)
#   MoveToDestination   BTTask_MoveToDestination_C (Mordhau/Content/Mordhau/AI/Tasks/BTTask_MoveToDestination)
class_name BtNavTasks
extends RefCounted

# ReceiveExecuteAI [1059]: EnemyPositions = the location of every BP_MordhauCharacter that is not dead, not our pawn
# and not (team mode and the same Team byte) [15..377]; GetRandomReachablePointInRadius(GetCentroid(EnemyPositions),
# 1000) - queried twice, the second result used [686, 754] -> blackboard TargetLocation, FinishExecute(true); no
# point -> ClearBlackboardValue(TargetLocation), FinishExecute(false). GetCentroid of an empty array: zero vector
# (UNCONFIRMED: engine library function not disassembled).
class FindRandomLocation extends BtTask:
	func execute(c) -> int:
		var pts := []
		for b in c.bodies:
			if b.is_dead or b == c.body or (c.team_mode and (b.team & 0xff) == (c.body.team & 0xff)):
				continue
			pts.append(b.location)
		var cen := Vector3.ZERO
		for p in pts:
			cen += p
		if not pts.is_empty():
			cen /= float(pts.size())
		var r := float(ModeKismet.kv("ai_find_random_radius"))
		if c.random_reachable_point(cen, r) != null:
			c.blackboard[params.target_location_key] = c.random_reachable_point(cen, r)
			c.note("FindRandomLocation", "target near the enemies' centroid", "BTTask_FindRandomLocation_C")
			return SUCCEEDED
		c.blackboard.erase(params.target_location_key)
		return FAILED

# ReceiveExecuteAI [26]: GetRandomReachablePointInRadius(pawn location, 500) - twice, the second result used ->
# TargetLocation, FinishExecute(true); none -> clear TargetLocation, FinishExecute(false)
class FindUnstuckSpot extends BtTask:
	func execute(c) -> int:
		var r := float(ModeKismet.kv("ai_unstuck_radius"))
		if c.random_reachable_point(c.body.location, r) != null:
			c.blackboard[params.target_location_key] = c.random_reachable_point(c.body.location, r)
			return SUCCEEDED
		c.blackboard.erase(params.target_location_key)
		return FAILED

# ReceiveExecuteAI [2985]: Init (controller, game state, pawn; bGoToActor = TargetActor key set), ResetTimeAndDistance.
# ReceiveTickAI [15]: pawn invalid -> FinishExecute(true); a ClimbingMotion -> return (keep waiting); controller
#   invalid -> Failed.
#   HasPath [384] (the controller is following a path):
#     GetTargetLoc moved more than 300 cm from MoveTargetLocation -> SetPath [567]; KeepMoving (target farther than
#     AcceptableRadius) else -> StopMovement, Succeeded [2539].
#     CurrentMovementDistance += |LastActorLocation - pawn|, LastActorLocation = pawn; > 100 -> ResetTimeAndDistance
#     [916]. CurrentMovementTime += dt; CurrentMovementTime + dt >= 2 (stuck) [1098]: a random reachable point
#     within 1000 of the pawn -> MoveToLocation(it), keep running; none -> FinishExecute(false) [1507]. Else after 0.5 s in an IdleMotion the frontal-hit door / destroyable handling [1565..2538]
#     (no doors / destroyables in the port: nothing).
#   no path [2601]: IsMovePending -> return; target farther than AcceptableRadius -> SetPath (keep running); else
#     FinishExecute(true). HasPath = the controller's path following is MOVING (UNCONFIRMED mapping).
# SetPath: StartFacingMovement(0), ResetTimeAndDistance, LastActorLocation = pawn; Stamina == 100 and (Health == 100
#   or !bAllowHealthRegen) and !ForceWalk -> StartSprinting else StopSprinting; MoveTargetLocation = GetTargetLoc;
#   UseMidpoint ? MoveToLocationWithRandomMidpoint : MoveToLocation (acceptance -1); result 0 (Failed) ->
#   FinishExecute(false), 1 (AlreadyAtGoal) -> FinishExecute(true), 2 -> keep going.
# ResetTimeAndDistance: time 0, distance 0, CurrentMovementLocation = pawn; facing mode != 0 -> StartFacingMovement(0).
class MoveToDestination extends BtTask:
	var go_to_actor := false
	var last_actor_location := Vector3.ZERO
	var move_target_location := Vector3.ZERO
	var current_movement_time := 0.0
	var current_movement_distance := 0.0
	var allow_health_regen := true		# MordhauGameState bAllowHealthRegen (the scene sets the mode's value)
	var _result := -1

	func execute(c) -> int:
		go_to_actor = c.blackboard.get(params.target_actor_key) is BotBody
		_reset(c)
		return IN_PROGRESS

	func _target(c) -> Vector3:
		var a = c.blackboard.get(params.target_actor_key)
		if go_to_actor and a is BotBody:
			return a.location
		var v = c.blackboard.get(params.target_location_key)
		return v if v is Vector3 else Vector3.ZERO

	func _reset(c) -> void:
		current_movement_time = 0.0
		current_movement_distance = 0.0
		if c.body != null and c.facing_mode != MordhauBotController.Facing.MOVEMENT:
			c.start_facing_movement(0.0)

	func _set_path(c) -> int:
		c.start_facing_movement(0.0)
		_reset(c)
		last_actor_location = c.body.location
		var full := int(ModeKismet.kv("ai_moveto_sprint_full"))
		if c.body.stamina_byte() == full and (c.body.health_byte() == full or not allow_health_regen) and not params.force_walk:
			c.start_sprinting()
		else:
			c.body.wants_sprint = false
		move_target_location = _target(c)
		var r: int = c.move_to_location_with_random_midpoint(move_target_location, -1.0) if params.use_midpoint \
			else _plain_move(c, move_target_location)
		if r == 0:
			return FAILED
		if r == 1:
			return SUCCEEDED
		return IN_PROGRESS

	func _plain_move(c, dest: Vector3) -> int:
		c.move_to_location(dest, -1.0)
		return 2

	func _keep_moving(c) -> bool:
		return (c.body.location - _target(c)).length_squared() > params.acceptable_radius * params.acceptable_radius

	func tick(c, dt: float) -> int:
		if c.body == null or c.body.is_dead:
			return SUCCEEDED
		if c.body.sys != null and c.body.sys.motion != null and c.body.sys.motion.kind() == "Climbing":
			return IN_PROGRESS
		if c.get_move_status() == MordhauBotController.PathStatus.MOVING:		# HasPath
			var rp := float(ModeKismet.kv("ai_moveto_repath_dist"))
			if _target(c).distance_squared_to(move_target_location) > rp * rp:
				var r := _set_path(c)
				if r != IN_PROGRESS:
					return r
			if not _keep_moving(c):
				c.stop_movement()
				return SUCCEEDED
			current_movement_distance += (last_actor_location - c.body.location).length()
			last_actor_location = c.body.location
			if current_movement_distance > float(ModeKismet.kv("ai_moveto_progress_dist")):
				_reset(c)
			current_movement_time += dt
			if current_movement_time + dt >= float(ModeKismet.kv("ai_moveto_stuck_s")):
				var ur := float(ModeKismet.kv("ai_moveto_unstuck_radius"))
				if c.random_reachable_point(c.body.location, ur) != null:
					c.move_to_location(c.random_reachable_point(c.body.location, ur), -1.0)
					return IN_PROGRESS
				c.note("MoveToDestination", "stuck, no unstuck point -> FinishExecute(false)", "BTTask_MoveToDestination_C")
				return FAILED
			return IN_PROGRESS
		if c.move_pending:
			return IN_PROGRESS
		if _keep_moving(c):
			return _set_path(c)
		return SUCCEEDED

	func abort(c) -> int:
		if c.body != null:
			c.stop_movement()
		return ABORTED

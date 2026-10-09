# bot_controller.gd - AMordhauAIController: owns the bot's BehaviorProfile, its behavior tree and blackboard, and turns
# the tasks' decisions into fighter input (MotionSystem.request_attack / request_parry / request_feint, the same entry
# points a player's input uses) plus movement / facing / crouch requests the scene applies.
# Source: extract/native/decomp/AMordhauAIController.cpp, layout extract/native/types/AMordhauAIController.h.
#
# Drop-in use (see README.md): one controller per bot; every frame, after the combat step, call tick(dt).
class_name MordhauBotController
extends RefCounted

enum Facing { MOVEMENT = 0, LOCATION = 1, ACTOR = 2, ACTOR_2D = 3, BONE = 4 }	# +0x36c, set by the StartFacing* below
enum PathStatus { IDLE = 0, WAITING = 1, PAUSED = 2, MOVING = 3 }			# EPathFollowingStatus (UE 4.26)

const DEFAULT_TREE := "BT_Deathmatch"		# BP_MordhauAIController CDO BehaviorTree (extract/json), used by make()

var body: BotBody
var bodies: Array = []			# every character in the world; perception stand-in (see perceived_enemies)
var profile: BotBehaviorProfile
var rng: UeRand
var tree: BtTree
var blackboard := {}
var world_state := {}			# shared between the controllers of one world (VoiceOrEmote cooldowns)
var clock: Callable				# () -> float world seconds
var combat						# optional CombatState: decisions are echoed into its event trace
var team_mode := false			# the world's AMordhauGameState bIsTeamMode (+0x6b0), read by perception
var nav_raycast: Callable		# (from: Vector3, to: Vector3) -> bool blocked; default: no navmesh, never blocked
# AMordhauAIController state
var _net_seen = null			# +0x328 last seen NetMotion Id (here: the identity of MotionSystem.net, see below)
var motion_random := 0.0		# +0x32c
var facing_mode := Facing.MOVEMENT
var facing_actor: BotBody = null	# +0x33c weak ptr (StartFacingActor / StartFacingBone)
var facing_location := Vector3.ZERO	# +0x35c
var facing_offset := Vector2.ZERO	# +0x354 (yaw, pitch offset for StartFacingBone / Actor)
var facing_param := 0.0			# +0x368: StartFacingMovement / StartFacingActor argument, StartFacingBone bone height
var facing_since := 0.0			# +0x404
var move_request := {}			# last AAIController::MoveToLocation {dest, acceptance, t}
var path_status := PathStatus.IDLE	# AAIController::GetMoveStatus: the scene sets IDLE when a move completes
var closest_enemy_override: BotBody = null	# SetClosestEnemyOverride rva=0x1515b50
var last_voice_time := 0.0		# voice component +0xe8 (initial value UNCONFIRMED)
var events: Array = []			# voice / emote / equip requests for the scene
var last_request := {}			# last fighter input the bot sent {kind, t, move, angle, bt}
var trace: Array = []			# decision trace: {t, task, step, fn}

# plain construction; make() loads profile and tree from the game's data
func _init(b: BotBody, all_bodies: Array, prof: BotBehaviorProfile, shared_rng: UeRand, bt: BtTree) -> void:
	body = b
	bodies = all_bodies
	profile = prof
	rng = shared_rng
	tree = bt
	clock = func() -> float: return b.sys.now()
	b.ai = self

# with data loaded (BotData, components/ue): the usual way to make a bot. Profile defaults from the BOTBEHAVIOR_*
# Blueprint chain, the tree package (BT_Deathmatch = BP_MordhauAIController CDO BehaviorTree; "" = no tree), and the
# body's MaxWalkSpeed from BP_MordhauCharacter CharMoveComp.
static func make(b: BotBody, all_bodies: Array, behavior := "BOTBEHAVIOR_Knight", shared_rng: UeRand = null,
		tree_name := DEFAULT_TREE) -> MordhauBotController:
	if b.sys != null:
		b.max_walk_speed = BotData.max_walk_speed()
	var c := MordhauBotController.new(b, all_bodies, BotBehaviorProfile.new(BotData.profile(behavior)),
		shared_rng if shared_rng != null else UeRand.new(), BtTree.new(BotData.tree_def(tree_name)) if tree_name != "" else null)
	c.begin_play()
	return c

func now() -> float:
	return clock.call()

func note(task: String, step: String, fn: String) -> void:
	trace.append({"t": now(), "task": task, "step": step, "fn": fn})
	if combat != null:
		combat.trace_event("%s bot %s: %s" % [body.name, task, step])

# one AI frame: the BT component tick (BtTree.tick follows UBehaviorTreeComponent::TickComponent's order)
func tick(dt: float) -> void:
	if body == null or body.is_dead or tree == null or logic_paused:
		return
	_random_float_tick()
	update_perception()
	tree.tick(self, dt)

# ---- AMordhauAIController --------------------------------------------------------------------------------------

# from AMordhauAIController::GetMotionBasedRandom rva=0x14fc660: when the pawn's MotionSystem NetMotion Id (+0xc4)
# differs from the one seen last (+0x328): remember it, MotionRandom (+0x32c) = FRand, BehaviorProfile->
# RerollRandomInstanceValues(). Returns +0x32c (disasm 0x1414fc6ef).
# The Id is incremented on every AssignNetMotion / AssignNetMotionSimple (UMotionSystemComponent.cpp: `+0xc4 + 1`),
# not on the return to idle (ChangeMotion). The combat port builds a new `net` Dictionary in every _assign and edits
# the dynamic param in place, so a new `net` object = a new Id.
func get_motion_based_random() -> float:
	if body.sys != null and not body.is_dead:
		var n = body.sys.net
		if not is_same(n, _net_seen):
			_net_seen = n
			motion_random = rng.frand()
			profile.reroll(rng)
			note("Controller", "new own motion -> reroll (random %.4f)" % motion_random, "AMordhauAIController::GetMotionBasedRandom rva=0x14fc660")
	return motion_random

# ---- perception (AMordhauAIController PerceivedCharacters +0x390: TMap<TWeakObjectPtr<AAdvancedCharacter>,
# FPerceptionInfo {bSight +0, bHearing +1, bDamage +2, Team +3, UpdateTime +4}) ----------------------------------
# The engine's UAIPerceptionComponent (senses configured by the ctor, below) calls OnPerceptionUpdated with the actors
# whose stimuli changed; UpdatePerceptionInfo reads each actor's last sensed stimuli. The engine side (UAISense_Sight /
# Hearing / Damage producing stimuli: UAISense_Sight::Update etc., engine code not disassembled) is not ported: the
# port's stimulus source is `stimuli` (adapter-supplied), default a fresh sight stimulus from every other live
# character (UNCONFIRMED stand-in = everyone in sight), and tick() reports every character as updated each AI frame
# (UNCONFIRMED: the engine reports only changes; OnPerceptionUpdated's 1 s PerceptionUpdateInterval gate keeps the
# per-entry refresh rate the same).
# Sense configs, AMordhauAIController ctor rva=0x14e3bf0 (stores into the SenseConfigSight / SenseConfigHearing /
# SenseConfigDamage subobjects; BP_MordhauAIController's subobjects override none; UE 4.26 field names UNCONFIRMED):
#   sight   MaxAge (+0x2c) 15, SightRadius (+0x50) 2600, LoseSightRadius (+0x54) 3000, PeripheralVisionAngle (+0x58) 65,
#           detects enemies / neutrals / friendlies (+0x5c bits 1|4|2)
#   hearing MaxAge 15, HearingRange (+0x50) 2000, LoSHearingRange (+0x54) 400
#   damage  MaxAge 30
#   PerceptionUpdateInterval (+0x334) 1, NotPerceivedTimeToForget (+0x338) 5 (read nowhere in the ported functions)
const SIGHT_RADIUS := 2600.0
const LOSE_SIGHT_RADIUS := 3000.0
const PERIPHERAL_VISION_ANGLE := 65.0
const SIGHT_MAX_AGE := 15.0
const HEARING_MAX_AGE := 15.0
const HEARING_LOS_RANGE := 400.0
const DAMAGE_MAX_AGE := 30.0
const PERCEPTION_UPDATE_INTERVAL := 1.0
var perceived := {}					# BotBody -> {sight, hearing, damage, team, update_time} (FPerceptionInfo)
var stimuli: Callable				# (BotBody) -> [{sense: "sight"|"hearing"|"damage", age, strength, receiver, location}]
var hearing_trace_blocked: Callable	# (from, to) -> bool: UWorld::LineTraceTestByChannel(ECC 3) for hearing beyond 400 cm
var perception_events: Array = []	# OnStartedPerceivingCharacter / OnStoppedPerceivingCharacter (BP events)
var _died_seen := {}				# characters whose OnCharacterDied this controller already handled

# FAIStimulus age test as UpdatePerceptionInfo rva=0x151e630 inlines it: age = Strength (+8) > 0 ? Age (+0) : FLT_MAX;
# sensed when age < MaxAge (MaxAge 0 -> FLT_MAX)
static func _stim_sensed(st: Dictionary, max_age: float) -> bool:
	var age: float = float(st.age) if float(st.strength) > 0.0 else INF
	return age < (max_age if max_age != 0.0 else INF)

func _default_stimuli(b: BotBody) -> Array:
	return [{"sense": "sight", "age": 0.0, "strength": 1.0, "receiver": body.location, "location": b.location}]

# AMordhauAIController::UpdatePerceptionInfo rva=0x151e630: for each of the character's last sensed stimuli, by
# sense: sight -> bSight = sensed(SightConfig MaxAge); hearing -> bHearing = sensed(HearingConfig MaxAge), and a heard
# stimulus farther than LoSHearingRange (squared distance receiver -> stimulus location) whose visibility trace is
# blocked is not heard; damage -> bDamage = sensed(DamageConfig MaxAge). Then Team = the character's Team byte,
# UpdateTime = world TimeSeconds. A sense with no stimulus keeps its old flag.
func update_perception_info(b: BotBody, info: Dictionary) -> void:
	var sts: Array = stimuli.call(b) if stimuli.is_valid() else _default_stimuli(b)
	for st in sts:
		match String(st.sense):
			"sight":
				info.sight = _stim_sensed(st, SIGHT_MAX_AGE)
			"hearing":
				info.hearing = _stim_sensed(st, HEARING_MAX_AGE)
				if info.hearing:
					var rcv: Vector3 = st.receiver
					var loc: Vector3 = st.location
					var far := (rcv - loc).length_squared() > HEARING_LOS_RANGE * HEARING_LOS_RANGE
					if far and hearing_trace_blocked.is_valid() and hearing_trace_blocked.call(rcv, loc):
						info.hearing = false
			"damage":
				info.damage = _stim_sensed(st, DAMAGE_MAX_AGE)
	info.team = b.team & 0xff
	info.update_time = _world_time()

func _world_time() -> float:
	return now() + real_time_offset

# AMordhauAIController::OnPerceptionUpdated rva=0x150cdd0:
#   1. each updated actor that is an AAdvancedCharacter, not our pawn, bCanBeDamaged and not yet in the map: bind its
#      OnCharacterDied / OnCharacterDestroyed to OnCharacterDiedOrDestroyed, add {false x3, Team 255, UpdateTime 0},
#      remember it as newly perceived
#   2. every entry: a gone character -> removed; else when PerceptionUpdateInterval <= now - UpdateTime:
#      UpdatePerceptionInfo; no sense left -> unbind, removed, OnStoppedPerceivingCharacter unless newly perceived;
#      still sensed and newly perceived -> OnStartedPerceivingCharacter
# bCanBeDamaged stand-in: the character is alive (UNCONFIRMED: the flag's writers are not ported).
func on_perception_updated(updated: Array) -> void:
	if body == null:
		return
	var fresh := []
	for b in updated:
		if b == null or b == body or b.is_dead or perceived.has(b):
			continue
		perceived[b] = {"sight": false, "hearing": false, "damage": false, "team": 0xff, "update_time": 0.0}
		fresh.append(b)
	var t := _world_time()
	for b in perceived.keys():
		if not bodies.has(b):
			perceived.erase(b)
			continue
		var info: Dictionary = perceived[b]
		if PERCEPTION_UPDATE_INTERVAL <= t - float(info.update_time):
			update_perception_info(b, info)
			if not (info.sight or info.hearing or info.damage):
				perceived.erase(b)
				if not fresh.has(b):
					perception_events.append({"kind": "stopped", "who": b.name})
			elif fresh.has(b):
				perception_events.append({"kind": "started", "who": b.name})

# AMordhauAIController::OnCharacterDiedOrDestroyed rva=0x1508160 (bound to the character's OnCharacterDied /
# OnCharacterDestroyed): unbind both, and if the character is in the map: remove it, OnStoppedPerceivingCharacter
func on_character_died_or_destroyed(b: BotBody) -> void:
	if b == null:
		return
	if perceived.has(b):
		perceived.erase(b)
		perception_events.append({"kind": "stopped", "who": b.name})

# one perception frame: the death delegates of perceived characters, then the stand-in perception update
func update_perception() -> void:
	for b in perceived.keys():
		if b.is_dead and not _died_seen.has(b):
			_died_seen[b] = true
			on_character_died_or_destroyed(b)
	for b in _died_seen.keys():
		if not b.is_dead:
			_died_seen.erase(b)
	on_perception_updated(bodies)

# GetPerceivedEnemies rva=0x14fd110 / GetPerceivedAllies rva=0x14fce70: a walk over PerceivedCharacters, keeping a
# valid AMordhauCharacter key whose info is perceived (bSight || bHearing || bDamage) and then, by team:
#   enemies (disasm 0x1414fd2fa-0x1414fd30f): GameState bIsTeamMode (+0x6b0) == 0 -> every perceived character;
#            else only info.Team (+3) != our pawn's Team (AAdvancedCharacter +0x660)
#   allies  (disasm 0x1414fcf05 / 0x1414fd06a-0x1414fd075): empty unless bIsTeamMode; else info.Team == our Team
func perceived_enemies() -> Array:
	var out := []
	for b in _perceived_characters():
		if not team_mode or int(perceived[b].team) != (body.team & 0xff):
			out.append(b)
	return out

func perceived_allies() -> Array:
	var out := []
	if not team_mode:
		return out
	for b in _perceived_characters():
		if int(perceived[b].team) == (body.team & 0xff):
			out.append(b)
	return out

# AMordhauAIController::GetClosestEnemy rva=0x14fa570 (Ghidra C + disassembly for the return paths):
#   1. ClosestEnemyOverride (+0x408) set, alive (+0x504 bIsDead clear) and not in ClosestEnemyIgnoreSet (+0x410) -> it
#   2. no pawn -> null. LastClosestEnemy (+0x3f4) gone or dead -> cleared, LastClosestEnemyChangedTime (+0x400) = 0
#   3. our pawn's motion is an UAttackMotion, or (ChangedTime + 1 > RealTimeSeconds and LastClosestEnemy not ignored)
#      -> LastClosestEnemy unchanged (re-evaluated at most once a second, never mid-attack)
#   4. otherwise, over the perceived characters: in team mode an ally whose controller is an AMordhauAIController
#      contributes its own LastClosestEnemy (when we perceive that one too) together with the ally's distance to it;
#      any other perceived character contributes itself. Each live, not ignored candidate gets an entry in
#      EnemyWithAllyCountMap (+1 when the ally is closer to it than we are) and the nearest one (squared distance
#      between root locations) is the closest enemy. If the closest has more allies on it than BehaviorProfile
#      IgnoreEnemiesWithAllyCount (no profile: 1), the nearest entry with <= that many replaces it, unless it is
#      160000 (400 cm squared) or more farther (bIsClosestEnemySaturated +0x3fc = true: keep the closest).
#      ReallyCloseEnemyCached (+0x3ec) = the result when its distance < 160000 else null; LastClosestEnemy = result
#      (ChangedTime = now on a change). Returns the result.
# Perception stand-in: perceived_enemies / perceived_allies (see above). ClosestEnemyIgnoreSet is never filled by the
# exe outside the ctor (UNCONFIRMED: no other writer found in extract/native/decomp); kept empty unless set.
var last_closest_enemy: BotBody = null			# +0x3f4
var last_closest_enemy_changed_time := 0.0		# +0x400 (UWorld RealTimeSeconds base, see real_time)
# The exe compares +0x400 with UWorld RealTimeSeconds (+0x5a0): the level's running time, already past 1 s when bots
# spawn into a loaded level. The port's clocks start at 0 with the match, so RealTimeSeconds = now() +
# real_time_offset (port adaptation, offset = the 1 s recheck interval so a bot sees enemies from its first frame;
# the real level time at the first spawn is UNCONFIRMED)
var real_time_offset := BotConstants.closest_enemy_recheck_s
var really_close_enemy: BotBody = null			# +0x3ec ReallyCloseEnemyCached
var closest_enemy_saturated := false			# +0x3fc bIsClosestEnemySaturated
var closest_enemy_ignore: Array = []			# +0x410 ClosestEnemyIgnoreSet

func closest_enemy() -> BotBody:
	var ov := closest_enemy_override
	if ov != null and not ov.is_dead and not closest_enemy_ignore.has(ov):
		return ov
	if body == null:
		return null
	if last_closest_enemy == null or last_closest_enemy.is_dead:
		last_closest_enemy = null
		last_closest_enemy_changed_time = 0.0
	var t := now() + real_time_offset
	var attacking := body.sys != null and body.sys.motion is AttackMotion
	if attacking:
		return last_closest_enemy
	if last_closest_enemy_changed_time + BotConstants.closest_enemy_recheck_s > t \
			and not closest_enemy_ignore.has(last_closest_enemy):
		return last_closest_enemy
	var counts := {}			# EnemyWithAllyCountMap
	var order := []
	var best: BotBody = null
	var best_d := INF
	for b in _perceived_characters():
		var cand: BotBody = b
		var ally_d := INF
		if team_mode and int(perceived[b].team) == (body.team & 0xff):
			cand = null
			var ac = b.ai
			if ac != null and ac.last_closest_enemy != null and _perceives(ac.last_closest_enemy):
				cand = ac.last_closest_enemy
				ally_d = (b.location - cand.location).length_squared()
		if cand == null or cand.is_dead or closest_enemy_ignore.has(cand):
			continue
		if not counts.has(cand):
			counts[cand] = 0
			order.append(cand)
		var dd := (body.location - cand.location).length_squared()
		if ally_d < dd:
			counts[cand] += 1
		if dd < best_d:
			best = cand
			best_d = dd
	var max_allies := profile.p.ignore_enemies_with_ally_count if profile != null else 1
	closest_enemy_saturated = false
	var pick := best
	var pick_d := best_d
	if best != null and int(counts[best]) > max_allies:
		pick = null
		pick_d = INF
		for e in order:
			if int(counts[e]) <= max_allies:
				var de := (body.location - (e as BotBody).location).length_squared()
				if de < pick_d:
					pick = e
					pick_d = de
		if best_d + BotConstants.closest_enemy_saturation_sq <= pick_d:
			closest_enemy_saturated = true
			pick = best
			pick_d = best_d
	really_close_enemy = pick if pick_d < BotConstants.closest_enemy_saturation_sq else null
	if last_closest_enemy != pick:
		last_closest_enemy = pick
		last_closest_enemy_changed_time = t
	return pick

# PerceivedCharacters entries that are valid and sensed (bSight || bHearing || bDamage), map order
func _perceived_characters() -> Array:
	var out := []
	for b in perceived:
		var i: Dictionary = perceived[b]
		if bodies.has(b) and (i.sight or i.hearing or i.damage):
			out.append(b)
	return out

func _perceives(b: BotBody) -> bool:
	return _perceived_characters().has(b)

# AMordhauAIController::GetClosestAlly rva=0x14fa3b0: no AAdvancedCharacter pawn -> null; else the perceived ally
# (GetPerceivedAllies) nearest by squared root-location distance. Dead allies are not skipped (no bIsDead test).
func closest_ally() -> BotBody:
	if body == null:
		return null
	var best: BotBody = null
	var bd := INF
	for b in perceived_allies():
		var d: float = (b.location - body.location).length_squared()
		if d < bd:
			bd = d
			best = b
	return best

# AMordhauAIController::GetKthClosestOfThree rva=0x14fb780: logs "Call to unimplemented function
# GetKthClosestOfThree()" and returns null
func kth_closest_of_three(_idx: int) -> BotBody:
	return null

# AMordhauAIController::GetTeam rva=0x14fd6b0: the MordhauPlayerState's Team, else the AAdvancedCharacter pawn's
# Team byte (+0x660), else -2. player_state_team: the adapter's MordhauPlayerState Team (null = none)
var player_state_team = null
func get_team() -> int:
	if player_state_team != null:
		return int(player_state_team)
	if body != null:
		return body.team & 0xff
	return -2

# AMordhauAIController::PerceivesAlly rva=0x15122b0 / PerceivesEnemy rva=0x15122e0: GetPerceivedAllies / Enemies
# not empty
func perceives_ally() -> bool:
	return perceived_allies().size() > 0

func perceives_enemy() -> bool:
	return perceived_enemies().size() > 0

# from AMordhauAIController::GetAllyClearanceSides rva=0x14fa080: 1 = only the left side is clear, 0 = only the right,
# otherwise a coin flip min(int(FRand * 2), 1) == 1 (_DAT_14432474c = 2/32767). The per-ally box test (allies within
# sqrt(_DAT_14433f71c) in the pawn's local frame) is not ported: with allies present the result is UNCONFIRMED.
func ally_clearance_sides() -> int:
	if not perceived_allies().is_empty():
		note("Controller", "ally clearance box test not ported; allies treated as clear", "GetAllyClearanceSides rva=0x14fa080")
	var r := mini(int(float(rng.rand()) * BotConstants.ally_clearance_rand_scale), 1)
	return 1 if r == 1 else 0

func get_move_status() -> int:
	return path_status

func currently_facing_actor() -> BotBody:
	return facing_actor

# StartFacingMovement rva=0x1516640: mode 0, actor cleared, offset zero, +0x368 = arg
func start_facing_movement(arg: float) -> void:
	facing_mode = Facing.MOVEMENT
	facing_actor = null
	facing_offset = Vector2.ZERO
	facing_param = arg

# StartFacingLocation rva=0x15165c0: mode 1, actor cleared, +0x35c = location, offset zero, +0x368 = 0
func start_facing_location(p: Vector3) -> void:
	facing_mode = Facing.LOCATION
	facing_actor = null
	facing_location = p
	facing_offset = Vector2.ZERO
	facing_param = 0.0

# StartFacingActor rva=0x15164a0: mode 2, +0x368 = arg, +0x404 = now, actor, offset
func start_facing_actor(a: BotBody, arg: float, offset: Vector2) -> void:
	facing_mode = Facing.ACTOR
	facing_actor = a
	facing_param = arg
	facing_offset = offset
	facing_since = now()

# StartFacingBone rva=0x1516520: mode 4, actor = the mesh's owner, bone height, offset, +0x404 = now
func start_facing_bone(a: BotBody, height: float, offset: Vector2) -> void:
	facing_mode = Facing.BONE
	facing_actor = a
	facing_param = height
	facing_offset = offset
	facing_since = now()

# StartFacingActor2D rva=0x1516430: FacingUpOffset (+0x368) = offset, LastFacingActorChangeTime (+0x404) = world
# TimeSeconds, FacingActor = actor, FacingOffset zero, mode 3
func start_facing_actor_2d(a: BotBody, up_offset: float) -> void:
	facing_param = up_offset
	facing_since = now()
	facing_actor = a
	facing_offset = Vector2.ZERO
	facing_mode = Facing.ACTOR_2D

# StopMovement rva=0x1516810: bMovePending (+0x5b8) = false, PathFollowingComponent->AbortMove (the scene's path
# following stops: IDLE)
var move_pending := false
func stop_movement() -> void:
	move_pending = false
	path_status = PathStatus.IDLE

# PauseLogic rva=0x1512210 / ResumeLogic rva=0x1515100: forward to the BrainComponent's virtuals at vtable +0x428 /
# +0x430 (UBrainComponent::PauseLogic / ResumeLogic in UE 4.26: names UNCONFIRMED, no UBrainComponent header); the
# port pauses the behavior tree tick. No brain component (no tree) -> nothing.
var logic_paused := false
func pause_logic(_reason := "") -> void:
	if tree != null:
		logic_paused = true

func resume_logic(_reason := "") -> void:
	if tree != null:
		logic_paused = false

# AAIController::MoveToLocation(dest, acceptance, ...): recorded for the scene's path following
func move_to_location(dest: Vector3, acceptance: float) -> void:
	move_request = {"dest": dest, "acceptance": acceptance, "t": now()}
	path_status = PathStatus.MOVING

func nav_blocked(a: Vector3, b: Vector3) -> bool:
	return nav_raycast.is_valid() and nav_raycast.call(a, b)

# ---- RandomFloat (+0x468) / NextRandomFloatAssignment (+0x46c) -------------------------------------------------
# AMordhauAIController::BeginPlay rva=0x14f0ba0: RandomFloat = FRand (rand() & 0x7fff) * (1/32767), then
# NextRandomFloatAssignment = rand * (60/32767) + 120 + TimeSeconds. AMordhauAIController::LODTick rva=0x14fe7c0, first
# thing: when Next < TimeSeconds (strictly): RandomFloat = FRand, Next = rand * (60/32767) + 60 + TimeSeconds.
# The port calls begin_play() from make() (the controller's BeginPlay) and _random_float_tick() at the top of tick()
# (LODTick's call site in AMordhauAIController::Tick and its LOD gating are not ported: UNCONFIRMED every frame).
var random_float := 0.0
var next_random_float_assignment := 0.0

func begin_play() -> void:
	random_float = float(rng.rand() & 0x7fff) * BotConstants.frand_scale
	next_random_float_assignment = float(rng.rand() & 0x7fff) * BotConstants.random_float_next_scale \
		+ BotConstants.random_float_first_delay + _world_time()

func _random_float_tick() -> void:
	var t := _world_time()
	if next_random_float_assignment <= t and t != next_random_float_assignment:
		random_float = float(rng.rand() & 0x7fff) * BotConstants.frand_scale
		next_random_float_assignment = float(rng.rand() & 0x7fff) * BotConstants.random_float_next_scale \
			+ BotConstants.random_float_delay + t

# ---- moves with a random midpoint ------------------------------------------------------------------------------
# nav_random_point: UNavigationSystemV1::GetRandomReachablePointInRadius stand-in (the navmesh is engine data the
# port does not build): (origin: Vector3, radius: float) -> Vector3, or null when no point is found. Not set: null.
var nav_random_point: Callable
const MID_POINT_ACCEPTANCE_RADIUS := 10.0	# AMordhauAIController ctor rva=0x14e3bf0 MidPointAcceptanceRadius (+0x600)
var pending_move := {}						# PendingReq (+0x5c0): {dest, acceptance}

func random_reachable_point(origin: Vector3, radius: float):
	return nav_random_point.call(origin, radius) if nav_random_point.is_valid() else null

# AMordhauAIController::GetMoveMidpoint_Implementation rva=0x14fc710: the pawn-goal midpoint, then a random reachable
# point within half the pawn-midpoint distance of it; no navigation system / no point -> the goal itself
func get_move_midpoint(goal: Vector3) -> Vector3:
	if body != null:
		var here := body.location
		var mid := (goal - here) * BotConstants.move_midpoint_half + here
		var p = random_reachable_point(mid, (here - mid).length() * BotConstants.move_midpoint_half)
		if p != null:
			return p
	return goal

# AMordhauAIController::MoveToLocationWithRandomMidpoint rva=0x14ffea0: abort the current move; build the query with
# a midpoint (BuildPathfindingQueryWithMidpoint rva=0x14f1be0, not ported: its result is taken as "a midpoint was
# found" when a navigation stand-in is set, UNCONFIRMED); none -> bMovePending = false and a plain MoveTo(dest); else bMovePending = true, PendingReq =
# the goal request, and MoveTo(midpoint) with MidPointAcceptanceRadius. Returns the move's request result
# (EPathFollowingRequestResult: 0 Failed, 1 AlreadyAtGoal, 2 RequestSuccessful; the port's moves always succeed).
func move_to_location_with_random_midpoint(dest: Vector3, acceptance: float) -> int:
	if path_status != PathStatus.IDLE:
		path_status = PathStatus.IDLE				# PathFollowingComponent->AbortMove
	if body == null or not nav_random_point.is_valid():
		move_pending = false
		move_to_location(dest, acceptance)
		return 2
	var mid := get_move_midpoint(dest)
	move_pending = true
	pending_move = {"dest": dest, "acceptance": acceptance}
	move_to_location(mid, MID_POINT_ACCEPTANCE_RADIUS)
	return 2

# AMordhauAIController::OnMoveCompleted rva=0x150cd90: the base OnMoveCompleted, then a pending goal request is
# issued (bMovePending = false, MoveTo(PendingReq)). The scene's path following calls this when it arrives.
func on_move_completed() -> void:
	path_status = PathStatus.IDLE
	if move_pending:
		move_pending = false
		move_to_location(pending_move.dest, pending_move.acceptance)

# ---- CanSee ----------------------------------------------------------------------------------------------------
# AMordhauAIController::CanSee rva=0x14f2530 (return paths from the disassembly, 0x1414f2976 / 0x1414f297a):
# trace (UWorld::LineTraceSingleByChannel, channel 0x12, ignoring the pawn and its two held items) from the pawn's
# eyes toward the target's root, `distance` long; the first hit actor (a PhysicsProxy's owner) being the target ->
# true; otherwise a second trace to that end point raised 100 cm (UpVector * 100); its hit being the target -> true;
# else false. distance < 0: an AMordhauActor target's +0x364 + 50 (not ported: such targets are interactables).
# trace_first_hit: (from, to, ignore: Array) -> the BotBody hit first, or null (the adapter's physics query).
var trace_first_hit: Callable

func can_see(target: BotBody, distance: float) -> bool:
	if body == null or target == null or distance < 0.0 or not trace_first_hit.is_valid():
		return false
	var eye := body.location + Vector3(0, 0, body.eye_height)
	var dir := BtTask.safe_normal(target.location - eye)
	var end := eye + dir * distance
	if trace_first_hit.call(eye, end, [body]) == target:
		return true
	return trace_first_hit.call(eye, end + Vector3(0, 0, BotConstants.can_see_raise), [body]) == target

# ---- the pawn (AMordhauCharacter) -------------------------------------------------------------------------------

# AMordhauCharacter::RequestAttack rva=0x15644c0 (EAttackMove, float angle) -> the fighter's input entry point
func request_attack(move: int, angle: float) -> void:
	last_request = {"kind": "attack", "t": now(), "move": move, "angle": angle}
	body.sys.request_attack(move, angle)

# AMordhauCharacter::RequestParry rva=0x1564860 (EBlockType, bool) -> the fighter's input entry point. The bool
# (always 1 from the bot tasks) is not modelled by the combat port.
func request_parry(bt: int) -> void:
	last_request = {"kind": "parry", "t": now(), "bt": bt}
	body.sys.request_parry(bt)

# AMordhauCharacter::RequestFeint rva=0x15646b0
func request_feint() -> void:
	last_request = {"kind": "feint", "t": now()}
	body.sys.request_feint()

func start_sprinting() -> void:
	body.wants_sprint = true

func start_crouching() -> void:
	body.wants_crouch = true

func stop_crouching() -> void:
	body.wants_crouch = false

# Where the scene should turn the bot (AMordhauAIController::LODTick rva=0x14fe7c0 does this in the game; it is not
# ported, so this is a plain reading of the facing state: UNCONFIRMED). Returns a UE-space point, or null to face
# the movement direction.
func facing_target():
	match facing_mode:
		Facing.LOCATION:
			return facing_location
		Facing.ACTOR, Facing.ACTOR_2D, Facing.BONE:
			if facing_actor != null:
				return facing_actor.location + Vector3(0, 0, facing_param if facing_mode == Facing.BONE else 0.0)
	return null

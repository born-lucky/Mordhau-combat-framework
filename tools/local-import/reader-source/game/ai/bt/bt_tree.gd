# bt_tree.gd - a behavior-tree runner for the game's own trees (BT_Deathmatch -> BT_CombatGeneric), built from typed
# node records (BotData.tree_def) read from BehaviorTree packages in extract/json (composites, decorators, services,
# tasks and their parameters; the defaults behind absent properties are cited on the BotData fields).
#
# UE's BehaviorTree runtime is engine code; only what these trees use is ported. Sources, by rule:
#   R1 Selector: next child after a failure, done on success / Sequence: next after a success, done on failure.
#      UE 4.26 docs "Behavior Tree Node Reference: Composites" (docs.unrealengine.com/4.26/en-US/InteractiveExperiences/
#      ArtificialIntelligence/BehaviorTrees/BehaviorTreeNodeReference/BehaviorTreeNodeReferenceComposites/).
#      Handlers in the exe: UBTComposite_Selector::GetNextChildHandler rva=0x38ab4f0 (not disassembled).
#   R2 A child whose decorators fail counts as Failed; the child's decorators are then told the node was processed
#      (OnNodeProcessed), which is where ForceSuccess turns it into Succeeded. CONFIRMED: UBTCompositeNode::
#      FindChildToExecute rva=0x387c820 (disasm 0x14387c8c3: LastResult = 1, then each decorator with the
#      bNotifyProcessed flag gets vtable +0x2f0 with &LastResult); UBTDecorator_ForceSuccess::OnNodeProcessed
#      rva=0x1b6e110 writes 0 (Succeeded).
#   R3 Cooldown passes when now - LastUseTimestamp >= CoolDownTime (UBTDecorator_Cooldown::CalculateRawConditionValue
#      rva=0x389c0c0, `comiss; setae`); LastUseTimestamp starts at -FLT_MAX (InitializeMemory rva=0x38af6b0) and is
#      set to now when the decorated node deactivates (OnNodeDeactivation rva=0x38b2b20). CONFIRMED.
#   R4 Blackboard decorator: bool key "Is Set" (OperationType 0) / "Is Not Set" (1); float key arithmetic
#      Equal 0, NotEqual 1, Less 2, LessOrEqual 3, Greater 4, GreaterOrEqual 5 (UE 4.26 EArithmeticKeyOperation;
#      the packages' CachedDescription strings agree: OperationType 4 = "Is Greater Than"). UNCONFIRMED: engine enum.
#   R5 One execution request per BT tick: a task that finishes at once (ExecuteTask != InProgress) calls
#      OnTaskFinished -> RequestExecution, which ends in ScheduleNextTick, not in ProcessExecutionRequest
#      (CONFIRMED: disasm of OnTaskFinished rva=0x3885cd0 and RequestExecution rva=0x388a2c0). So the next task runs on
#      the next tick. Walking through composites and failed decorators happens inside one request.
#   R6 Tick order inside UBehaviorTreeComponent::TickComponent rva=0x388e990: (a call not resolved by the PDB labels,
#      read as the auxiliary-node tick), ProcessExecutionRequest, parallel tasks, then WrappedTickTask on the active
#      task - so a task that just went latent is ticked in the same frame. CONFIRMED call order; the identity of the
#      first call is UNCONFIRMED.
#   R7 Services: Interval 0.5, RandomDeviation 0.1 (UBTService::UBTService rva=0x3895d00), next tick after
#      max(0, I - D) + FRand * ((I + D) - max(0, I - D)) (UBTService::ScheduleNextTick rva=0x38b8190, one rand()).
#      UNCONFIRMED: a service ticks once when its composite becomes active (UE 4.26 "newly added aux nodes are ticked
#      as part of search"; flags not decoded).
#   R8 Observer aborts: a decorator with FlowAbortMode LowerPriority whose condition becomes true while a lower-priority
#      sibling branch runs aborts that branch (AbortTask) and re-runs its parent from the decorated child. UE docs
#      "Behavior Tree Overview: Decorators / Observer Aborts". UNCONFIRMED: re-evaluated each tick, not on key change.
#   R9 When the root composite finishes the tree starts again from the root, at most once per tick. UNCONFIRMED.
#   R10 Wait: remaining = max(0, W - D) + FRand * ((W + D) - max(0, W - D)) (UBTTask_Wait::ExecuteTask rva=0x38a5d40),
#      InProgress; each tick remaining -= dt, Succeeded once remaining <= 0 (TickTask rva=0x38bb8c0). CONFIRMED.
#      RandomDeviation defaults to 0 when the package has none (UNCONFIRMED: ctor rva=0x3897ca0 not decoded).
#   R11 RunBehavior runs the referenced tree in place; its result is the subtree root's result. UE docs (Run Behavior).
#
# Blueprint tasks: BTTask_BackOff_C (bt_back_off.gd) and BTTask_FindRandomLocation_C / FindUnstuckSpot_C /
# MoveToDestination_C (bt_nav_tasks.gd) are ported from their bytecode (scripts/kismet on the raw packages); any other
# Blueprint task fails (UNCONFIRMED stand-in). BTService_DMPerceptionUpdate_C, from its bytecode
# (ExecuteUbergraph_BTService_DMPerceptionUpdate [132..455]): GetClosestEnemy valid -> bPerceivesEnemy = true and
# ClosestEnemyDistance = VSize(enemy - pawn); else ClearBlackboardValue(bPerceivesEnemy) (a cleared bool reads false).
class_name BtTree
extends RefCounted

const SUCCEEDED := 0
const FAILED := 1
const ABORTED := 2
const IN_PROGRESS := 3

class BtNode:
	var name := ""
	var type := ""
	var def: BotData.BtNodeDef
	var kind := ""					# "selector" | "sequence" | "task" | "subtree"
	var children: Array = []		# BtNode
	var child_decorators: Array = []	# Array per child of BtDecorator
	var services: Array = []		# BtService
	var task: BtTask
	var subtree: BtNode				# RunBehavior: the referenced tree's root composite

class BtDecorator:
	var name := ""
	var type := ""
	var def: BotData.BtAux
	var last_use := -3.4028234663852886e38	# Cooldown memory (R3)
	func abort_mode() -> String:
		return def.flow_abort_mode

class BtService:
	var name := ""
	var type := ""
	var def: BotData.BtAux
	var next_time := 0.0

# BTTask_Wait (R10)
class BtWait extends BtTask:
	var remaining := 0.0
	func execute(c) -> int:
		var w := params.wait_time
		var dv := params.random_deviation
		var lo := maxf(w - dv, 0.0)
		remaining = float(c.rng.rand()) * BotConstants.bt_wait_rand_scale * ((w + dv) - lo) + lo
		return IN_PROGRESS
	func tick(_c, dt: float) -> int:
		remaining -= dt
		return SUCCEEDED if remaining <= 0.0 else IN_PROGRESS

# a task whose code is not in extract/ (Blueprint without bytecode) or not ported (ranged)
class BtUnported extends BtTask:
	var result := 1
	func execute(c) -> int:
		c.note("BT", "%s not ported -> %s" % [type_name, RESULT_NAMES[result]], "")
		return result

# ported native tasks by UE class name (engine-neutral: a name -> constructor table)
static func native_task(type: String) -> BtTask:
	match type:
		"BTTask_MeleeDefend": return BtMeleeDefend.new()
		"BTTask_MeleeAttack": return BtMeleeAttack.new()
		"BTTask_FallForFeint": return BtFallForFeint.new()
		"BTTask_SwitchEquipment": return BtSwitchEquipment.new()
		"BTTask_VoiceOrEmote": return BtVoiceOrEmote.new()
		"BTTask_Wait": return BtWait.new()
		"BTTask_BackOff_C": return BtBackOff.new()
		"BTTask_FindRandomLocation_C": return BtNavTasks.FindRandomLocation.new()
		"BTTask_FindUnstuckSpot_C": return BtNavTasks.FindUnstuckSpot.new()
		"BTTask_MoveToDestination_C": return BtNavTasks.MoveToDestination.new()
	return null

# Blueprint tasks without bytecode: result they stand in with
const UNPORTED_RESULT := {}

var asset := ""
var root: BtNode
var bb_types := {}				# blackboard key -> "Bool" | "Float" | "Object" | "Vector" ...
var task_overrides := {}		# node type -> Script (BtTask) supplied by the scene, e.g. a navmesh MoveTo
# execution state
var stack: Array = []			# frames {node: BtNode, idx: int, via: BtNode (RunBehavior node) or null}
var active: BtNode = null		# latent task
var pending := true
var pending_result := -1
var pending_child: BtNode = null
var _restarted_tick := -1
var _tick_n := 0

# def: the tree's typed records (BotData.tree_def); overrides: node type -> Callable () -> BtTask
func _init(def: BotData.TreeDef, overrides := {}) -> void:
	task_overrides = overrides
	asset = def.asset
	bb_types = def.blackboard
	root = _build(def.root)

func _build(d: BotData.BtNodeDef) -> BtNode:
	var n := BtNode.new()
	n.name = d.name
	n.type = d.type
	n.def = d
	n.kind = d.kind
	if n.kind == "subtree":
		n.subtree = _build(d.subtree)
	elif n.kind == "task":
		n.task = _make_task(n)
	for se in d.services:
		var sv := BtService.new()
		sv.name = se.name
		sv.type = se.type
		sv.def = se
		n.services.append(sv)
	for ch in d.children:
		n.children.append(_build(ch.node))
		var decs := []
		for de in ch.decorators:
			var dd := BtDecorator.new()
			dd.name = de.name
			dd.type = de.type
			dd.def = de
			decs.append(dd)
		n.child_decorators.append(decs)
	return n

func _make_task(n: BtNode) -> BtTask:
	var t: BtTask = task_overrides[n.type].call() if task_overrides.has(n.type) else native_task(n.type)
	if t == null:
		var u := BtUnported.new()
		u.result = UNPORTED_RESULT.get(n.type, FAILED)
		t = u
	return t.setup(n.name, n.type, n.def.params)

# ---- running ---------------------------------------------------------------------------------------------------

func tick(c, dt: float) -> void:
	_tick_n += 1
	_tick_services(c)										# R6: auxiliary nodes first
	_check_aborts(c)										# R8
	if pending:
		_search(c)											# R5
	if active != null:										# R6: the active task, also one that just went latent
		var r := active.task.tick(c, dt)
		if r != IN_PROGRESS:
			_finish(c, active, r)

func _finish(c, n: BtNode, r: int) -> void:
	c.note("BT", "%s finished %s" % [n.name, BtTask.RESULT_NAMES[r]], "")
	active = null
	pending = true											# OnTaskFinished -> RequestExecution -> next tick (R5)
	pending_result = r
	pending_child = n

func _frame_decorators(fr: Dictionary, idx: int) -> Array:
	return fr.node.child_decorators[idx] if idx >= 0 and idx < fr.node.child_decorators.size() else []

# the decorated child deactivates: OnNodeProcessed (ForceSuccess) and OnNodeDeactivation (Cooldown stamp)
func _deactivate(c, decs: Array, r: int) -> int:
	for d in decs:
		if d.type == "BTDecorator_ForceSuccess":
			r = SUCCEEDED
		elif d.type == "BTDecorator_Cooldown":
			d.last_use = c.now()
	return r

func _push(c, n: BtNode, via: BtNode) -> void:
	stack.append({"node": n, "idx": -1, "via": via})
	for s in n.services:
		_service_tick(c, s)									# R7 first tick on activation (UNCONFIRMED)

func _search(c) -> void:
	pending = false
	var res := pending_result
	if pending_child != null and not stack.is_empty():
		res = _deactivate(c, _frame_decorators(stack.back(), stack.back().idx), res)
	pending_child = null
	for _guard in 1000:
		if stack.is_empty():
			if _restarted_tick == _tick_n:					# R9: once per tick
				pending = true
				pending_result = -1
				return
			_restarted_tick = _tick_n
			c.note("BT", "start %s" % asset.get_file(), "")
			_push(c, root, null)
			res = -1
			continue
		var fr: Dictionary = stack.back()
		var n: BtNode = fr.node
		var nxt := -1
		if fr.idx < 0:
			nxt = 0 if n.children.size() > 0 else -1
			if n.children.is_empty():
				res = FAILED								# empty composite (UNCONFIRMED)
		elif n.kind == "selector":
			nxt = fr.idx + 1 if res != SUCCEEDED and fr.idx + 1 < n.children.size() else -1	# R1
		else:
			nxt = fr.idx + 1 if res == SUCCEEDED and fr.idx + 1 < n.children.size() else -1	# R1
		if nxt < 0:
			stack.pop_back()
			if not stack.is_empty():
				res = _deactivate(c, _frame_decorators(stack.back(), stack.back().idx), res)
			continue
		fr.idx = nxt
		var ch: BtNode = n.children[nxt]
		if not _allowed(c, _frame_decorators(fr, nxt)):
			res = FAILED									# R2
			for d in _frame_decorators(fr, nxt):
				if d.type == "BTDecorator_ForceSuccess":
					res = SUCCEEDED
			continue
		match ch.kind:
			"selector", "sequence":
				_push(c, ch, null)
				res = -1
			"subtree":
				c.note("BT", "%s -> %s" % [ch.name, ch.def.subtree_name], "")
				_push(c, ch.subtree, ch)
				res = -1
			_:
				var r := ch.task.execute(c)
				c.note("BT", "execute %s -> %s" % [ch.name, BtTask.RESULT_NAMES[r]], "")
				if r == IN_PROGRESS:
					active = ch
					return
				pending = true								# R5
				pending_result = r
				pending_child = ch
				return
	c.note("BT", "search did not settle (1000 steps)", "")

func _allowed(c, decs: Array) -> bool:
	for d in decs:
		if not _condition(c, d):
			return false
	return true

func _condition(c, d: BtDecorator) -> bool:
	var ok := true
	match d.type:
		"BTDecorator_Cooldown":
			ok = c.now() - d.last_use >= d.def.cooldown_time		# R3
		"BTDecorator_Blackboard":
			var key := d.def.blackboard_key
			var op := d.def.operation_type
			var val = c.blackboard.get(key)
			match bb_types.get(key, ""):
				"Bool":
					ok = bool(val) if op == 0 else not bool(val)						# R4
				"Float":
					var x := float(val) if val != null else 0.0
					var y := d.def.float_value
					ok = [x == y, x != y, x < y, x <= y, x > y, x >= y][op] if op < 6 else false
				_:
					ok = val != null if op == 0 else val == null
			if d.def.b_inverse_condition:
				ok = not ok
		"BTDecorator_ForceSuccess":
			ok = true
		_:
			c.note("BT", "decorator %s not ported (passes)" % d.type, "")
	return ok

# R8: a LowerPriority observer on an earlier child of a running composite whose condition now holds
func _check_aborts(c) -> void:
	for si in stack.size():
		var fr: Dictionary = stack[si]
		for i in range(0, fr.idx):
			var decs := _frame_decorators(fr, i)
			var observing := false
			for d in decs:
				var m: String = d.abort_mode()
				if m == "LowerPriority" or m == "Both":
					observing = true
			if observing and _allowed(c, decs):
				c.note("BT", "abort lower priority: %s" % fr.node.children[i].name, "")
				if active != null:
					active.task.abort(c)
					active = null
				while stack.size() > si + 1:
					stack.pop_back()
				fr.idx = i - 1
				pending = true
				pending_result = FAILED
				pending_child = null
				return

func _tick_services(c) -> void:
	for fr in stack:
		for s in fr.node.services:
			if c.now() >= s.next_time:
				_service_tick(c, s)

func _service_tick(c, s: BtService) -> void:
	match s.type:
		"BTService_DMPerceptionUpdate_C":
			# BTService_DMPerceptionUpdate_C bytecode (header)
			var en: BotBody = c.closest_enemy()
			c.blackboard[s.def.perceives_enemy_key] = en != null
			if en != null:
				c.blackboard[s.def.closest_enemy_distance_key] = \
					(en.location - c.body.location).length()
		_:
			pass		# BTService_ObstacleNavigator_C (navigation, Blueprint without bytecode): not ported
	# R7 UBTService::ScheduleNextTick
	var iv := s.def.interval
	var dv := s.def.random_deviation
	var lo := maxf(iv - dv, 0.0)
	s.next_time = c.now() + float(c.rng.rand()) * ((iv + dv) - lo) * BotConstants.bt_service_rand_scale + lo

# bot_data.gd - the bot's data adapter: the only place the bot (game/ai) gets the extract/ readers (UePkg, NativeCtor,
# CombatData) from. It turns the game's data into plain values the engine-agnostic logic consumes
# (the .rdata literals are in bot_constants.gd, BotConstants):
#   profile_def  merged UBotBehaviorProfile defaults (native ctor + Blueprint CDO chain) as a Dictionary
#   tree_def     a BehaviorTree package (plus its RunBehavior subtrees and blackboard key types) as typed node records
#   profile      a typed UBotBehaviorProfile (class data + instance rolls), one copy per bot
# Everything in game/ai is plain data + deterministic tick functions: porting the logic to another engine means
# rewriting this adapter (and BotConstants) and replacing Vector3 (used only as a 3-float value) with that engine's vector.
class_name BotData

const BT_ROOT := "Mordhau/Content/Mordhau/AI/BehaviorTrees/"
const BP_TASKS := "Mordhau/Content/Mordhau/AI/Tasks/"
const PROFILE_ROOT := "Mordhau/Content/Mordhau/Blueprints/BotProfiles/BotBehaviorProfiles/"

static func profile_def(name_or_path: String) -> Dictionary:
	if name_or_path == "":
		return NativeCtor.defaults("UBotBehaviorProfile").duplicate(true)
	var p := name_or_path if name_or_path.contains("/") else PROFILE_ROOT + name_or_path
	return CombatData.class_defaults(p).duplicate(true)

# ---- behavior trees ------------------------------------------------------------------------------------------------
# A BehaviorTree package as typed node records. Node exports hold only serialized properties; the class defaults
# behind them: Mordhau's native tasks (UBTTask_SwitchEquipment, UBTTask_VoiceOrEmote, ...) from their ctors
# (NativeCtor, merged under the node's properties, so their fields are required reads); engine nodes (no PDB layout
# here) from the UE ctors named on each field. Rules that use them: game/ai/bt/bt_tree.gd (R1..R11).

# a decorator or service
class BtAux:
	var name := ""
	var type := ""
	var flow_abort_mode := "None"		# UBTDecorator FlowAbortMode (EBTFlowAbortMode, ctor None)
	var b_inverse_condition := false	# UBTDecorator bInverseCondition (ctor false)
	var cooldown_time := 0.0			# BTDecorator_Cooldown CoolDownTime
	var blackboard_key := ""			# BTDecorator_Blackboard BlackboardKey.SelectedKeyName
	var operation_type := 0			# BTDecorator_Blackboard OperationType (EBasicKeyOperation / EArithmeticKeyOperation)
	var float_value := 0.0				# BTDecorator_Blackboard FloatValue
	var interval := 0.0				# UBTService Interval
	var random_deviation := 0.0		# UBTService RandomDeviation
	var perceives_enemy_key := ""		# BTService_DMPerceptionUpdate_C bPerceivesEnemy.SelectedKeyName
	var closest_enemy_distance_key := ""	# its ClosestEnemyDistance.SelectedKeyName

# a task's parameters (by task class)
class BtTaskParams:
	var wait_time := 0.0				# BTTask_Wait WaitTime
	var random_deviation := 0.0		# BTTask_Wait RandomDeviation
	var b_melee := false				# UBTTask_SwitchEquipment +0x70
	var allowed_subclasses := PackedStringArray()		# +0x78 class names (no _C)
	var not_allowed_subclasses := PackedStringArray()	# +0x88
	var chance := 0.0					# UBTTask_VoiceOrEmote +0x98
	var global_cooldown := 0.0			# +0x94
	var voice_commands := PackedInt32Array()	# +0x70
	var emotes := PackedInt32Array()			# +0x80
	var b_force_emote := false			# +0x90
	# Blueprint tasks (AI/Tasks/BTTask_*_C): blackboard selectors of the node instance, variables of the class CDO
	var target_location_key := ""		# TargetLocation.SelectedKeyName (FindRandomLocation, FindUnstuckSpot, MoveToDestination)
	var target_actor_key := ""			# TargetActor.SelectedKeyName (MoveToDestination; CDO "None")
	var acceptable_radius := 0.0		# BTTask_MoveToDestination AcceptableRadius (CDO 250)
	var use_midpoint := false			# BTTask_MoveToDestination UseMidpoint (CDO true)
	var force_walk := false				# BTTask_MoveToDestination ForceWalk (absent from the CDO: false)

class BtNodeDef:
	var name := ""
	var type := ""
	var kind := ""						# "selector" | "sequence" | "task" | "subtree"
	var children: Array[BtChildDef] = []
	var services: Array[BtAux] = []
	var subtree: BtNodeDef = null		# RunBehavior: the referenced tree's root composite
	var subtree_name := ""				# BehaviorAsset object name
	var params := BtTaskParams.new()

class BtChildDef:
	var node: BtNodeDef
	var decorators: Array[BtAux] = []

class TreeDef:
	var asset := ""
	var blackboard := {}				# blackboard key -> "Bool" | "Float" | "Object" | "Vector" ...
	var root: BtNodeDef

static func tree_def(name_or_path: String) -> TreeDef:
	var path := name_or_path if name_or_path.contains("/") else BT_ROOT + name_or_path
	var pkg := UePkg.load_pkg(path)
	var r := UeRec.new(UePkg.export_of(pkg, "BehaviorTree").get("Properties", null), path)
	var out := TreeDef.new()
	out.asset = path
	var bb := r.obj("BlackboardAsset")
	if bb != "":
		var br := UeRec.new(UePkg.export_of(UePkg.load_pkg(bb), "BlackboardData").get("Properties", null), bb, r.errors)
		for k in br.arr("Keys"):
			var kr := UeRec.new(k, bb + ".Keys[]", r.errors)
			var cls := String(kr.sub("KeyType").d.get("ObjectName", ""))
			if cls == "":
				r.errors.append("%s.Keys[]: KeyType without ObjectName" % bb)
			out.blackboard[kr.s("EntryName")] = cls.get_slice("'", 0).trim_prefix("BlackboardKeyType_")
	out.root = _node(pkg, r.ref_index("RootNode"), out.blackboard, path, r.errors)
	return r.done(out)

static func _ref_index(v, what: String, errors: Array) -> int:
	return UeRec.new({"ref": v}, what, errors).ref_index("ref")

static func _node(pkg: Array, i: int, keys: Dictionary, path: String, errors: Array) -> BtNodeDef:
	var n := BtNodeDef.new()
	if i < 0 or i >= pkg.size():
		errors.append("%s: node index %d out of range" % [path, i])
		return n
	var e: Dictionary = pkg[i]
	n.name = String(e.get("Name", ""))
	n.type = String(e.get("Type", ""))
	var r := UeRec.new(_with_native(n.type, e.get("Properties", {})), "%s.%s" % [path.get_file(), n.name], errors)
	match n.type:
		"BTComposite_Selector": n.kind = "selector"
		"BTComposite_Sequence": n.kind = "sequence"
		"BTTask_RunBehavior":
			n.kind = "subtree"
			var sub_path := r.obj("BehaviorAsset")
			n.subtree_name = String(r.sub("BehaviorAsset").d.get("ObjectName", ""))
			var sub := tree_def(sub_path)
			if sub != null:
				keys.merge(sub.blackboard)
				n.subtree = sub.root
		_:
			n.kind = "task"
			_task_params(n, r)
	for s in r.arr_or("Services"):
		n.services.append(_aux(pkg, _ref_index(s, r.what, errors), path, errors))
	for ch in r.arr_or("Children"):
		var cr := UeRec.new(ch, r.what + ".Children[]", errors)
		var c := BtChildDef.new()
		var ref := "ChildComposite" if cr.has("ChildComposite") and cr.d.ChildComposite != null else "ChildTask"
		c.node = _node(pkg, cr.ref_index(ref), keys, path, errors)
		for d in cr.arr_or("Decorators"):
			c.decorators.append(_aux(pkg, _ref_index(d, cr.what, errors), path, errors))
		n.children.append(c)
	return n

# native Mordhau task: its ctor defaults (NativeCtor) under the node's serialized properties
static func _with_native(type: String, props: Dictionary) -> Dictionary:
	if not NativeCtor.has_type("U" + type):
		return props
	var d: Dictionary = NativeCtor.defaults("U" + type).duplicate()
	for k in props:
		d[k] = props[k]
	return d

static func _class_names(r: UeRec, k: String) -> PackedStringArray:
	var out := PackedStringArray()
	for v in r.arr_or(k):			# TArray: absent = empty
		if not (v is Dictionary and v.has("ObjectName")):
			r.errors.append("%s.%s: expected class references" % [r.what, k])
			continue
		out.append(String(v.ObjectName).get_slice("'", 1).trim_suffix("_C"))
	return out

static func _task_params(n: BtNodeDef, r: UeRec) -> void:
	var t := n.params
	match n.type:
		"BTTask_Wait":
			# UBTTask_Wait ctor (engine, rva=0x3897ca0, not decoded): WaitTime 5.0, RandomDeviation 0 - UE 4.26
			# BTTask_Wait.cpp; UNCONFIRMED against the exe
			t.wait_time = r.f_or("WaitTime", 5.0)
			t.random_deviation = r.f_or("RandomDeviation", 0.0)
		"BTTask_SwitchEquipment":
			t.b_melee = r.b("bMelee")
			t.allowed_subclasses = _class_names(r, "AllowedSubclasses")
			t.not_allowed_subclasses = _class_names(r, "NotAllowedSubclasses")
		"BTTask_FindRandomLocation_C", "BTTask_FindUnstuckSpot_C", "BTTask_MoveToDestination_C":
			var cdo := UeRec.new(UePkg.cdo(UePkg.load_pkg(BP_TASKS + n.type.trim_suffix("_C"))), n.type, r.errors)
			var tl := r.sub("TargetLocation") if r.has("TargetLocation") else cdo.sub("TargetLocation")
			t.target_location_key = tl.s("SelectedKeyName")
			if n.type == "BTTask_MoveToDestination_C":
				var ta := r.sub("TargetActor") if r.has("TargetActor") else cdo.sub("TargetActor")
				t.target_actor_key = ta.s("SelectedKeyName")
				t.acceptable_radius = r.f("AcceptableRadius") if r.has("AcceptableRadius") else cdo.f("AcceptableRadius")
				t.use_midpoint = r.b("UseMidpoint") if r.has("UseMidpoint") else cdo.b("UseMidpoint")
				t.force_walk = r.b("ForceWalk") if r.has("ForceWalk") else cdo.b_or("ForceWalk", false)
		"BTTask_VoiceOrEmote":
			t.chance = r.f("Chance")
			t.global_cooldown = r.f("GlobalCooldown")
			t.b_force_emote = r.b("bForceEmote")
			for v in r.arr_or("VoiceCommandsList"):
				t.voice_commands.append(int(v))
			for v in r.arr_or("EmotesList"):
				t.emotes.append(int(v))

static func _aux(pkg: Array, i: int, path: String, errors: Array) -> BtAux:
	var a := BtAux.new()
	if i < 0 or i >= pkg.size():
		errors.append("%s: aux node index %d out of range" % [path, i])
		return a
	var e: Dictionary = pkg[i]
	a.name = String(e.get("Name", ""))
	a.type = String(e.get("Type", ""))
	var r := UeRec.new(e.get("Properties", {}), "%s.%s" % [path.get_file(), a.name], errors)
	a.flow_abort_mode = r.enum_or("FlowAbortMode", "None")
	a.b_inverse_condition = r.b_or("bInverseCondition", false)
	match a.type:
		"BTDecorator_Cooldown":
			# UBTDecorator_Cooldown ctor (engine): CoolDownTime 5.0 - UE 4.26 BTDecorator_Cooldown.cpp; UNCONFIRMED
			a.cooldown_time = r.f_or("CoolDownTime", 5.0)
		"BTDecorator_Blackboard":
			a.blackboard_key = r.sub("BlackboardKey").s("SelectedKeyName")
			a.operation_type = r.i_or("OperationType", 0)		# uint8, ctor 0 (Is Set / Equal)
			a.float_value = r.f_or("FloatValue", 0.0)
		"BTService_DMPerceptionUpdate_C":
			a.perceives_enemy_key = r.sub("bPerceivesEnemy").s("SelectedKeyName")
			a.closest_enemy_distance_key = r.sub("ClosestEnemyDistance").s("SelectedKeyName")
	if a.type.begins_with("BTService"):
		# UBTService::UBTService rva=0x3895d00: Interval 0.5, RandomDeviation 0.1 (CONFIRMED, bt_tree.gd R7)
		a.interval = r.f_or("Interval", 0.5)
		a.random_deviation = r.f_or("RandomDeviation", 0.1)
	return a

# ---- typed records -------------------------------------------------------------------------------------------------
# UBotBehaviorProfile (types/UBotBehaviorProfile.h): the class data and the per-instance rolls, all UPROPERTYs of the
# object, read from profile_def (native ctor rva=0x144de70 replayed by NativeCtor, then the BOTBEHAVIOR_* chain), so
# every field is present and read as required. A bot gets its own copy (copy()): Randomize and RerollRandomInstanceValues
# write into it.
class Profile:
	# class data
	var ignore_enemies_with_ally_count := 0				# +0x44
	var b_prefers_alt_mode := false						# +0x5c
	var back_off_factor_during_defense_min_max := Vector2.ZERO	# +0x60
	var base_attack_hesitance_time := 0.0				# +0x68
	var attack_hesitance_variance := 0.0				# +0x6c
	var footwork_instead_of_parry_probability := 0.0	# +0x70
	var footwork_with_crouch_probability := 0.0			# +0x74
	var parry_timing_variance := Vector2.ZERO			# +0x78
	var perfect_parry_probability := 0.0				# +0x80
	var feint_timing_variance := 0.0					# +0x84
	var fall_for_feint_probability := 0.0				# +0x88
	var out_of_range_feint_probability := 0.0			# +0x8c
	var combo_probability := 0.0						# +0x90
	var drag_probability := 0.0							# +0x94
	var accel_probability := 0.0						# +0x98
	var chamber_probability := 0.0						# +0x9c
	var morph_probability := 0.0						# +0xa0
	var gamble_probability := 0.0						# +0xa4
	var feint_probability := 0.0						# +0xa8
	var riposte_probability := 0.0						# +0xac
	var brawl_probability := 0.0						# +0xb0
	var max_turn_rate := 0.0							# +0xb4
	var max_look_up_rate := 0.0							# +0xb8
	# instance rolls (RerollRandomInstanceValues)
	var random_2d_unit_vector := Vector2.ZERO			# +0xbc
	var will_brawl := false								# +0xc4
	var will_riposte := false							# +0xc6
	var back_off_factor_during_defense := 0.0			# +0xc8
	var will_feint := false								# +0xcc
	var will_out_of_range_feint := false				# +0xcd
	var will_gamble := false							# +0xce
	var will_morph := false								# +0xcf
	var will_chamber := false							# +0xd0
	var will_accel := false								# +0xd1
	var will_drag := false								# +0xd2
	var will_combo := false								# +0xd3
	var will_fall_for_feint := false					# +0xd4
	var feint_timing_random := 0.0						# +0xd8
	var will_perfect_parry := false						# +0xdc
	var parry_timing_random := 0.0						# +0xe0
	var will_footwork := false							# +0xe4
	var will_footwork_with_crouch := false				# +0xe5
	var attack_hesitance_random := 0.0					# +0xf0

	func read(r: UeRec) -> void:
		ignore_enemies_with_ally_count = r.i("IgnoreEnemiesWithAllyCount")
		b_prefers_alt_mode = r.b("bPrefersAltMode")
		back_off_factor_during_defense_min_max = r.v2("BackOffFactorDuringDefenseMinMax")
		base_attack_hesitance_time = r.f("BaseAttackHesitanceTime")
		attack_hesitance_variance = r.f("AttackHesitanceVariance")
		footwork_instead_of_parry_probability = r.f("FootworkInsteadOfParryProbability")
		footwork_with_crouch_probability = r.f("FootworkWithCrouchProbability")
		parry_timing_variance = r.v2("ParryTimingVariance")
		perfect_parry_probability = r.f("PerfectParryProbability")
		feint_timing_variance = r.f("FeintTimingVariance")
		fall_for_feint_probability = r.f("FallForFeintProbability")
		out_of_range_feint_probability = r.f("OutOfRangeFeintProbability")
		combo_probability = r.f("ComboProbability")
		drag_probability = r.f("DragProbability")
		accel_probability = r.f("AccelProbability")
		chamber_probability = r.f("ChamberProbability")
		morph_probability = r.f("MorphProbability")
		gamble_probability = r.f("GambleProbability")
		feint_probability = r.f("FeintProbability")
		riposte_probability = r.f("RiposteProbability")
		brawl_probability = r.f("BrawlProbability")
		max_turn_rate = r.f("MaxTurnRate")
		max_look_up_rate = r.f("MaxLookUpRate")
		random_2d_unit_vector = r.v2("Random2DUnitVector")
		will_brawl = r.b("bWillBrawl")
		will_riposte = r.b("bWillRiposte")
		back_off_factor_during_defense = r.f("BackOffFactorDuringDefense")
		will_feint = r.b("bWillFeint")
		will_out_of_range_feint = r.b("bWillOutOfRangeFeint")
		will_gamble = r.b("bWillGamble")
		will_morph = r.b("bWillMorph")
		will_chamber = r.b("bWillChamber")
		will_accel = r.b("bWillAccel")
		will_drag = r.b("bWillDrag")
		will_combo = r.b("bWillCombo")
		will_fall_for_feint = r.b("bWillFallForFeint")
		feint_timing_random = r.f("FeintTimingRandom")
		will_perfect_parry = r.b("bWillPerfectParry")
		parry_timing_random = r.f("ParryTimingRandom")
		will_footwork = r.b("bWillFootwork")
		will_footwork_with_crouch = r.b("bWillFootworkWithCrouch")
		attack_hesitance_random = r.f("AttackHesitanceRandom")

	func copy() -> Profile:
		var c := Profile.new()
		for p in get_property_list():
			if p.usage & PROPERTY_USAGE_SCRIPT_VARIABLE:
				c.set(p.name, get(p.name))
		return c

static var _profiles := {}

# a fresh copy of a behavior profile's defaults ("" = the native UBotBehaviorProfile)
static func profile(name_or_path: String) -> Profile:
	if not _profiles.has(name_or_path):
		var r := UeRec.new(profile_def(name_or_path), "UBotBehaviorProfile(%s)" % name_or_path)
		var p := Profile.new()
		p.read(r)
		_profiles[name_or_path] = r.done(p)
	var src: Profile = _profiles[name_or_path]
	return src.copy() if src != null else null

# CharMoveComp MaxWalkSpeed (the body's walk speed; CharacterData.movement reads the template)
static func max_walk_speed() -> float:
	return CharacterData.movement().max_walk_speed

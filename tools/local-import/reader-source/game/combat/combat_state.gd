# combat_state.gd - the deterministic combat simulation: a fixed-step world clock, fighters (MotionSystem), input,
# per-tick weapon and body transforms, hit processing, a per-tick state trace and an event stream.
#
# Driving it from a game (the integration builder's Node3D characters):
#   var cs := CombatState.new(1.0 / 60.0)
#   cs.add_fighter("player", weapon_bp_path); cs.add_fighter("bot", weapon_bp_path)
#   cs.combat_event.connect(_on_combat_event)        # {kind: hit/parry/chamber/motion/died, attacker, victim, ...}
#   every physics frame:
#     cs.set_look_up("player", pitch_degrees)           # AAdvancedCharacter LookUpValue (clamped pitch, degrees)
#     cs.input_attack("player", CombatEnums.Move.RIGHT_STRIKE, angle_deg)   # / input_feint / input_parry / release
#     cs.set_weapon_root("player", <global Transform3D of the weapon skeleton's root bone "Armature">)
#     cs.set_bones("bot", {bone_name: global Transform3D, ...})            # UMA skeleton bones, Godot space
#     cs.step()
#   read cs.fighters[name].motion.kind(), .stamina, .health for animation/UI.
# Units: Godot metres / Y-up; data is converted at read time (UePhysics).
#
# Tick order per step n (t = n * dt), as one engine frame runs it:
#   1. queued inputs. Each fighter's controller is a tick prerequisite of its pawn: AController::AddPawnTickDependency
#      (exe 0x14301d3a0, disassembled) adds the controller's PrimaryActorTick as prerequisite of the pawn's movement
#      component and, unless that one ticks before its owner, of the pawn's PrimaryActorTick. UNCONFIRMED: that the
#      input handlers (BlockPressed, RequestAttack, ...) run inside the controller's tick (APlayerController::PlayerTick
#      reaches ProcessPlayerInput through a virtual call that was not resolved).
#   2. every fighter's tick (TG_PrePhysics): MotionSystem.tick = AMordhauCharacter::LODTick order (motion tick,
#      buffered parry, stamina regen), see motion_system.gd.
#   3. every fighter's late tick (TG_PostPhysics, ULateTickComponent): PrepareForTracing while attacking, then tracing
#      in Release, which processes hits.
#   4. scripted contacts (tests: a trace result handed in directly, processed like a traced hit).
# Within one tick group the fighters run in insertion order. The game code adds tick prerequisites only inside one
# character: AAdvancedCharacter::BeginPlay rva=0x1459110 makes the actor tick wait for its PrePhysTickFunction and, unless
# the console variable "m.NoLateTickPrereq" (string at 0x14431ba78) is set, its LateTickComponent wait for its own Mesh
# (AddTickPrerequisiteComponent); AMordhauEquipment's mesh waits for its owner's mesh (AMordhauEquipment.cpp 6557). No
# Mordhau function orders one character's tick after another's, so two characters' late ticks (and their traces) have
# no order the game sets; the port's insertion order is a simulation choice. It matters for a same-tick trade: a hit
# is applied at once inside the tracing character's late tick (ProcessHitForDamage AssignNetMotion on the authority),
# and an attack stays flinchable in Release (UAttackMotion::OnBegin_Implementation rva=0x162eda0 clears bIsFlinchable
# only for a riposte out of a non-shield-wall parry or a bIsUnflinchable owner), so the fighter processed first
# flinches the other, whose Release then no longer traces (test_combat_motion_gates
# test_same_tick_trade_order_decides). A riposte out of a parry is unflinchable, so both of those land.
class_name CombatState
extends RefCounted

signal combat_event(ev: Dictionary)

var dt := 1.0 / 1000.0
var tick_n := 0
var now := 0.0
var fighters := {}				# name -> MotionSystem
var schedule := {}				# tick -> [ {kind, who, ...} ]
var trace: Array = []			# one row per tick: {n, t, <name>: motion summary}
var events: PackedStringArray = PackedStringArray()
var hits: Array = []			# every emitted event dictionary, in order
var attack_traces_memory := []	# AMordhauGameState +0x530 AttackTracesMemory: [{start, end, destroy_time, owner}]
var mode_rules = null			# CombatData.ModeRules of the running AMordhauGameMode / AMordhauGameState; null = no game
								# mode (bare combat tests): the damage chain's game-mode / game-state branches are skipped
var record_trace := true		# false: no per-tick `trace` row (long runs, parity replays: the trace is O(ticks))
var authority := true			# false: a client's copy of the world (game/net NetClient). Hits are processed on the
								# authority only; the client copy learns their results through replication. UNCONFIRMED /
								# not ported: the owning client's own hit suggestion (AMordhauCharacter::
								# ServerSuggestHitDetection_Implementation rva=0x1567d80)

func _init(step_s := 1.0 / 1000.0) -> void:
	dt = step_s

# left_path: optional left-hand equipment (AMordhauCharacter LeftHandEquipment), e.g. a buckler with a one-hander
func add_fighter(nm: String, weapon_path: String, left_path := "") -> MotionSystem:
	var f := MotionSystem.new(self, nm, weapon_path, left_path)
	f.creation_time = now			# AActor CreationTime: the pawn spawns now
	fighters[nm] = f
	return f

# The game mode passes its Blueprints (e.g. BP_TeamDeathmatchGameMode + BP_TeamDeathmatchGameState): DamageFactor,
# TeamDamageFactor, TeamDamageFlinch, SpawnProtectionDuration, bDisableDamage, bIsHitStopOnTeamHitsDisabled, bIsTeamMode
# from their CDOs (CombatData.mode_rules). Fighter teams: MotionSystem.team (AMordhauPlayerState.Team, 255 = none).
func set_game_mode(game_mode_bp: String, game_state_bp: String) -> void:
	mode_rules = CombatData.mode_rules(game_mode_bp, game_state_bp)

# from AMordhauGameState::IsFriendly_Implementation rva=0x159bc10 (characters, bIsFriendlyIfSelf false): the same
# actor -> false; else both teams known (!= 0xff) and equal and the game state's bIsTeamMode. No game state -> false.
func is_friendly(a, b) -> bool:
	if mode_rules == null or a == null or b == null or a == b:
		return false
	var ta: int = int(a.team) & 0xff
	var tb: int = int(b.team) & 0xff
	return ta != 0xff and tb != 0xff and mode_rules.b_is_team_mode and ta == tb

# ---- live API (applied on the next step) --------------------------------------------------------------------
func input_attack(who: String, move: int, angle := 0.0) -> void:
	_push(tick_n + 1, {"kind": "attack", "who": who, "move": move, "angle": angle})

func input_feint(who: String) -> void:
	_push(tick_n + 1, {"kind": "feint", "who": who})

# block pressed (AMordhauCharacter::BlockPressed): parry now, retried every tick until it succeeds or is released
func input_parry(who: String, block_type := 0) -> void:
	_push(tick_n + 1, {"kind": "parry", "who": who, "bt": block_type})

func input_release_block(who: String) -> void:
	_push(tick_n + 1, {"kind": "release_block", "who": who})

func set_weapon_root(who: String, xf: Transform3D) -> void:
	fighters[who].tracer.root_xf = xf

func set_bones(who: String, bone_xf: Dictionary) -> void:
	fighters[who].bone_xf = bone_xf

# AAdvancedCharacter LookUpValue (+0x520) in degrees, up positive: the game accumulates look input into it and clamps
# it to [-LookDownLimit, LookUpLimit] (AAdvancedCharacter::LookUp rva=0x1489930). UStrikeMotion::IsInEarlyRelease
# divides max(value, 0) by LookUpLimit (BP_MordhauCharacter CDO 55) itself, so pass the clamped pitch, not a ratio.
func set_look_up(who: String, degrees: float) -> void:
	fighters[who].look_up_value = degrees

# AMordhauWeapon::SwitchMode (alternate grip), applied on the next step
func input_switch_mode(who: String) -> void:
	_push(tick_n + 1, {"kind": "switch_mode", "who": who})

# ---- scripted API (tests) -------------------------------------------------------------------------------------
func at_time(t: float) -> int:
	return int(round(t / dt))

func _push(n: int, ev: Dictionary) -> void:
	if not schedule.has(n):
		schedule[n] = []
	schedule[n].append(ev)

func attack(t: float, who: String, move: int, angle := 0.0) -> void:
	_push(at_time(t), {"kind": "attack", "who": who, "move": move, "angle": angle})

func feint(t: float, who: String) -> void:
	_push(at_time(t), {"kind": "feint", "who": who})

func parry(t: float, who: String, block_type := 0) -> void:
	_push(at_time(t), {"kind": "parry", "who": who, "bt": block_type})

func release_block(t: float, who: String) -> void:
	_push(at_time(t), {"kind": "release_block", "who": who})

# ToggleWeaponMode input (AMordhauCharacter::RequestToggleWeaponMode): raises a shield wall with a shield in the left
# hand, else switches the weapon's mode
func toggle_mode(t: float, who: String) -> void:
	_push(at_time(t), {"kind": "toggle_mode", "who": who})

func input_toggle_mode(who: String) -> void:
	_push(tick_n + 1, {"kind": "toggle_mode", "who": who})

func switch_mode(t: float, who: String) -> void:
	_push(at_time(t), {"kind": "switch_mode", "who": who})

# The attacker's weapon reaches the defender's `bone` at t without geometry (the trace result given directly):
# runs the same ProcessHitForBlocking / ProcessHitForDamage as a traced hit.
func contact(t: float, attacker: String, defender: String, bone := "Spine1") -> void:
	_push(at_time(t), {"kind": "contact", "who": attacker, "target": defender, "bone": bone})

# a callable run before the tick at t (tests use it to move weapons/bones)
func at(t: float, fn: Callable) -> void:
	_push(at_time(t), {"kind": "call", "fn": fn})

func run_until(t_end: float) -> void:
	while now < t_end - dt * 0.5:
		step()

func step() -> void:
	tick_n += 1
	now = tick_n * dt
	var evs: Array = schedule.get(tick_n, [])
	for ev in evs:
		if ev.kind == "call":
			ev.fn.call()
	for ev in evs:
		if ev.kind != "contact" and ev.kind != "call":
			_input(ev)
	_prune_trace_memory()
	for nm in fighters:
		fighters[nm].tick(dt)
	for nm in fighters:
		var f: MotionSystem = fighters[nm]
		f.late_tick()
		if authority:
			MeleeHit.trace_and_process(self, f)
	for ev in evs:
		if ev.kind == "contact" and authority:
			_contact(fighters[ev.who], fighters[ev.target], ev.bone)
	if not record_trace:
		return
	var row := {"n": tick_n, "t": now}
	for nm in fighters:
		var f: MotionSystem = fighters[nm]
		var r := {"kind": f.motion.kind(), "stamina": f.stamina, "health": f.health}
		r.merge(f.motion.trace_fields())
		row[nm] = r
	trace.append(row)

# AMordhauGameState::Tick rva=0x15ab790 (decomp AMordhauGameState.cpp, the loop over +0x530/+0x538): an entry whose
# DestroyTime (+0x18) is before now is removed. UNCONFIRMED: game-state tick before the characters' (tick groups of
# AMordhauGameState not traced); with a 1 ms step the order moves an entry's lifetime by one tick at most.
func _prune_trace_memory() -> void:
	var keep := []
	for e in attack_traces_memory:
		if not (float(e.destroy_time) < now):
			keep.append(e)
	attack_traces_memory = keep

func _input(ev: Dictionary) -> void:
	var f: MotionSystem = fighters[ev.who]
	match ev.kind:
		"attack": f.request_attack(ev.move, ev.angle)
		"feint": f.request_feint()
		"parry": f.block_pressed(ev.bt)
		"switch_mode": f.switch_mode()
		"toggle_mode": f.toggle_weapon_mode()
		"release_block": f.release_block()
	trace_event("%s input %s" % [ev.who, ev.kind])

func _contact(a: MotionSystem, b: MotionSystem, bone: String) -> void:
	var atk = a.motion
	if not (atk is AttackMotion) or atk.stage != CombatEnums.Stage.RELEASE:
		trace_event("%s contact ignored (not in Release)" % a.name)
		return
	MeleeHit.process_hit(self, a, b, bone)

func trace_event(s: String) -> void:
	events.append("%.3f %s" % [now, s])

func emit_event(ev: Dictionary) -> void:
	ev["t"] = now
	hits.append(ev)
	combat_event.emit(ev)

func trace_motion(f: MotionSystem) -> void:
	var m := f.motion
	var extra := ""
	if m is AttackMotion:
		extra = " %s %s windup_end=%.4f release_end=%.4f end=%.4f" % [CombatEnums.MOVE_NAMES[m.move],
			CombatEnums.ATTACK_TYPE_NAMES[m.type],
			m.windup_end, m.release_end, m.end_time]
	elif m is FeintedMotion or m is ParryMotion or m is BlockedMotion or m is FlinchMotion:
		extra = " end=%.4f" % m.end_time
	trace_event("%s -> %s%s" % [f.name, m.kind(), extra])
	emit_event({"kind": "motion", "who": f.name, "motion": m.kind()})

# Read-only tracer state per fighter (AMordhauWeapon fields, types/AMordhauWeapon.h), for the bot:
# {current_trace_start (+0xd48), current_trace_end (+0xd54), previous_trace_end (+0xd6c), current_valid (+0xd40),
#  previous_valid (+0xd41), actor_ignore_cache (+0xe88, fighter names), length (+0x1bec)}
func tracer_state(who: String) -> Dictionary:
	var t: WeaponTracer = fighters[who].tracer
	return {"current_trace_start": t.cur_start, "current_trace_end": t.cur_end, "previous_trace_end": t.prev_end,
		"current_valid": t.b_cur_valid, "previous_valid": t.b_prev_valid,
		"actor_ignore_cache": t.actor_ignore_cache.map(func(i): return name_of(i)), "length": t.length}

# fighter name of a MotionSystem.id ("" when that pawn has left the world)
func name_of(fighter_id: int) -> String:
	for nm in fighters:
		if fighters[nm].id == fighter_id:
			return nm
	return ""

# the fighter with that MotionSystem.id, null when it has left the world
func fighter_by_id(fighter_id: int) -> MotionSystem:
	for f in fighters.values():
		if f.id == fighter_id:
			return f
	return null

# Trace queries for tests
func first(who: String, pred: Callable) -> Dictionary:
	for row in trace:
		if pred.call(row[who]):
			return row
	return {}

# Ownership: the world owns its fighters (`fighters`), a fighter owns its motions and tracer; every back-pointer is a
# WeakRef (MotionSystem.world, CombatMotion.sys) and every fighter-to-fighter reference is a MotionSystem.id int (hit
# lists, ignore caches, trace memory owners), and the motion history is bounded (MotionSystem._bound_history). So
# the object graph has no reference cycles: dropping the last reference to a CombatState frees it and everything it
# owns. (The 13 GB parity-replay growth of round 5 came from cycles; test_combat_world_freed_without_dispose checks a
# whole bout is freed without calling anything.)
# dispose() stays for callers that want the memory back immediately while still holding the world, and to drop the
# per-fighter network link (MotionSystem.net_rep, game/net), which this layer does not own.
func dispose() -> void:
	for f in fighters.values():
		f.net_rep = null
	fighters.clear()
	schedule.clear()
	trace.clear()
	hits.clear()
	events.clear()
	attack_traces_memory.clear()

# Removes one fighter (a respawn between rounds): drops it from `fighters`, its pending inputs from `schedule`, its
# entries from attack_traces_memory, and its id from the other fighters' attack hit lists and tracer ignore cache.
# Nothing else in the world refers to it, so it is freed as soon as the caller lets go of it.
func remove_fighter(nm: String) -> void:
	var f = fighters.get(nm)
	if f == null:
		return
	fighters.erase(nm)
	for k in schedule:
		schedule[k] = schedule[k].filter(func(ev): return ev.get("who", "") != nm and ev.get("target", "") != nm)
	attack_traces_memory = attack_traces_memory.filter(func(e): return e.owner != f.id)
	for other in fighters.values():
		for m in [other.motion, other.last_attack_motion]:
			if m != null and m is AttackMotion:
				m.hit_actors.erase(f.id)
		other.tracer.actor_ignore_cache.erase(f.id)
	f.net_rep = null

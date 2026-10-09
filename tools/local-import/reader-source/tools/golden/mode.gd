# tools/golden/mode.gd - golden-trace runner for the game modes and bots (Rust port: core/crates/mh-mode), called by
# tools/export_golden.gd for every scenario of core/tests/scenarios/mode.json ("runner": "res://tools/golden/mode.gd").
# Output: core/tests/golden/mode/<name>.jsonl (git-ignored, Triternion data): the exporter's header line, then
#   kind "mode": {kind: "header", data: <ModeData record>, consts, skm_cp_def?} and one row per tick
#                {n, ops, events, cp_events, state}: `ops` are the host actions applied before that tick's mode.tick(dt)
#                (login / logout / kill / despawn / damage / alive / loc / overlap / unoverlap / skm_cp), recorded as
#                applied, so the Rust test replays them exactly; the scenario's "plan" only decides them here.
#   kind "bots": {kind: "header", bodies, bots (profile + tree records), consts} and one row per tick
#                {n, pre (every body's fighter view after the combat step), req (each bot request with the requester's
#                fighter view right after it), out (every bot's state), rng}: the Rust test feeds `pre` / `req` views to
#                its bots (the combat port is not in this crate) and compares every decision.
# Everything here is the reference (godot/game/mode, godot/game/ai, godot/game/combat) run unchanged; the plans are
# deterministic functions of the state.
extends RefCounted

var g						# the exporter (write_row)
var s: Dictionary
var m: MordhauGameMode
var ops: Array = []			# this tick's applied ops
var _despawn: Array = []	# names to despawn next tick (adapter: the dead pawn leaves after the frame)
var _k := 0					# plan counter
var _nets: Array = []		# NetMotion objects seen (identity -> index)

const LS := "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword"
const SKM_CP := "Mordhau/Content/Mordhau/Blueprints/GameModes/Skirmish/BP_SkirmishCapturePoint"
const CURVE_SAMPLES := 65	# capture curves at lead 0..64 (ModeData ControlPointDef capture_speed, mh-mode data.rs)

func run(sc: Dictionary, exporter) -> void:
	g = exporter
	s = sc
	if String(s.get("kind", "mode")) == "bots":
		_run_bots()
	elif String(s.get("kind", "mode")) == "data":
		_run_data()
	else:
		_run_mode()

# ---- JSON conversion ---------------------------------------------------------------------------------------------
static func _j(v):
	if v is Transform3D:
		return [v.origin.x, v.origin.y, v.origin.z]
	if v is Vector3:
		return [v.x, v.y, v.z]
	if v is Vector2:
		return [v.x, v.y]
	if v is Color:
		return [v.r, v.g, v.b, v.a]
	if v is Dictionary:
		var o := {}
		for k in v:
			o[String(k)] = _j(v[k])
		return o
	if v is Array or v is PackedStringArray or v is PackedInt32Array or v is PackedFloat32Array or v is PackedFloat64Array:
		var a := []
		for x in v:
			a.append(_j(x))
		return a
	if v is MordhauGameMode.Ctrl:
		return v.name
	if v is BotBody:
		return v.name
	if v is Object:
		return _obj(v)
	return v

# every script variable of a record object (typed records: ModeData.*, BotData.*), recursively
static func _obj(o: Object) -> Dictionary:
	var d := {}
	if o == null:
		return d
	for p in o.get_property_list():
		if p.usage & PROPERTY_USAGE_SCRIPT_VARIABLE:
			d[p.name] = _j(o.get(p.name))
	if o is ModeData.PlayerStartDef:
		d["origin"] = [o.xf.origin.x, o.xf.origin.y, o.xf.origin.z]
	if o is ModeData.ControlPointDef:
		var cs := []
		var ns := []
		for i in CURVE_SAMPLES:
			cs.append(CombatData.curve_value(o.capture_speed_curve, float(i)) if o.capture_speed_curve != "" else 0.0)
			ns.append(CombatData.curve_value(o.neutralize_speed_curve, float(i)) if o.neutralize_speed_curve != "" else 0.0)
		d["capture_speed"] = cs if o.capture_speed_curve != "" else []
		d["neutralize_speed"] = ns if o.neutralize_speed_curve != "" else []
	return d

static func _consts() -> Dictionary:
	# touch one static of each class first: the cites tables fill when the class's statics initialize
	var _touch := [ModeConstants.spawn_rand_scale, BotConstants.frand_scale]
	var mo := {}
	for k in ModeConstants.cites:
		mo[k] = ModeConstants.cites[k][2]
	var bo := {}
	for k in BotConstants.cites:
		bo[k] = BotConstants.cites[k][2]
	return {"mode": mo, "bot": bo}

# ---- modes -------------------------------------------------------------------------------------------------------
func _make_mode(id: String) -> MordhauGameMode:
	var r := UeRand.new()
	match id:
		"FFA": return FfaMode.new(FfaMode.FfaData.new(), r)
		"TDM": return TdmMode.new(TdmMode.TdmData.new(), r)
		"SKM": return SkmMode.new(SkmMode.SkmData.new(), r)
		"DU": return DuelMode.new(DuelMode.DuelData.new(), r)
		"TF": return TfMode.new(TfMode.TfData.new(), r)
		"FL":
			var fdat := FlMode.FlData.new()
			_patch_fl(fdat)
			return FlMode.new(fdat, r)
	push_error("golden/mode: no mode %s" % id)
	return null

# scenario "fl_patch": {"team1_starting_tickets": x, "team2_starting_tickets": x, "points": {name: {"owning_team": t}}}
# overrides of the map's records, so a short scenario reaches Frontline's end rules (ticket exhaustion, zero points);
# the header records the patched data, so the Rust replay builds the same mode
func _patch_fl(fdat) -> void:
	var p: Dictionary = s.get("fl_patch", {})
	for k in ["team1_starting_tickets", "team2_starting_tickets"]:
		if p.has(k):
			fdat.set(k, float(p[k]))
	var pts: Dictionary = p.get("points", {})
	for def in fdat.control_points:
		if pts.has(def.name):
			for k in pts[def.name]:
				def.set(k, pts[def.name][k])

func _run_mode() -> void:
	var id := String(s.mode)
	m = _make_mode(id)
	var data := _obj(m.d)
	var ext := data.duplicate()
	ext["id"] = id
	data["ext"] = ext
	var hdr := {"kind": "header", "data": data, "consts": _consts(), "dt": float(s.dt), "ticks": int(s.ticks)}
	if String(s.get("plan", "")) == "skm_cp":
		hdr["skm_cp_def"] = _obj(ModeData.control_point(SKM_CP))
	# the first tick's control point events (BeginPlay) come before any tick: row n = -1
	g.write_row(hdr)
	g.write_row({"n": -1, "ops": [], "events": _js(m.drain()), "cp_events": _cp_events(), "state": _state()})
	var dt := float(s.dt)
	var pv: MordhauPlayerView = null
	var dv: DuelPlayerView = null
	for n in int(s.ticks):
		ops = []
		_plan(n)
		m.tick(dt)
		var evs := m.drain()
		# the local player's views (HUD side): MordhauPlayerView for "player", plus DuelPlayerView in room modes
		var hud := []
		if pv == null and m.by_name("player") != null:
			pv = MordhauPlayerView.new(m, m.by_name("player"))
			if m is DuelMode:
				dv = DuelPlayerView.new(m.by_name("player"), (m as DuelMode).dd)
		if pv != null:
			pv.on_events(evs)
			pv.tick()
			hud.append_array(pv.drain())
		if dv != null:
			dv.tick(m.now, m.now)
			hud.append_array(dv.drain())
		g.write_row({"n": n, "ops": ops, "events": _js(evs), "cp_events": _cp_events(), "state": _state(), "hud": _js(hud)})

static func _js(evs: Array) -> Array:
	var out := []
	for e in evs:
		out.append(_j(e))
	return out

func _cp_events() -> Array:
	var out := []
	for cp in m.control_points:
		out.append_array(_js(cp.drain()))
	return out

func _names(a: Array) -> Array:
	var out := []
	for c in a:
		out.append(c.name if c != null else "")
	return out

func _rg(g0: Dictionary) -> Array:
	if g0.is_empty():
		return []
	return [g0.team1_wins, g0.team2_wins, g0.round.stage, g0.round.winner, g0.round.start_time]

func _state() -> Dictionary:
	var st := {"now": m.now, "ms": m.match_state, "el": m.elapsed_time, "sd": m.scoring_disabled,
		"as": m.allow_spawning, "tro": m.match_time_ran_out, "ts": m.team_scores.duplicate(),
		"cp1": m.team1_capture_points, "cp2": m.team2_capture_points, "rng": [m.rng.seed, m.rng.calls],
		"q": _names(m.spawns.queue), "sp": m.spawns.spawning.name if m.spawns.spawning != null else "",
		"step": m.spawns.step, "end": _j(m.match_end_info), "sb": m.scoreboard_seconds()}
	var cs := []
	var mr := {}
	for c in m.controllers:
		var dh := []
		for e in c.damage_history:
			dh.append([e.who.name, e.t, e.damage])
		var row := {"name": c.name, "team": c.team, "alive": c.alive, "score": c.score, "k": c.kills, "d": c.deaths,
			"a": c.assists, "pawn": c.has_pawn, "nrt": c.next_respawn_time, "wr": c.wants_respawn,
			"laf": c.last_asked_for_spawn, "pid": c.player_id, "cpt": c.capture_point_time,
			"cpi": m.control_points.find(c.capture_point) if c.capture_point != null else -1,
			"loc": _j(c.location), "blk": m.should_block_input(c), "dh": dh, "mid": c.match_id,
			"iiw": _names(c.in_instance_with), "rrg": _rg(c.replicated_room_game) if c.has_replicated else []}
		cs.append(row)
		if not m.match_end_info.is_empty():
			mr[c.name] = _j(m.match_result_for(c, m.match_end_info))
	st["c"] = cs
	st["mr"] = mr
	if m is SkmMode:
		var k: SkmMode = m
		st["skm"] = {"ri": [k.round_info.stage, k.round_info.winner, k.round_info.start_time],
			"rpp": k.round_points_per_team.duplicate(), "srp": k.state_round_points.duplicate(),
			"ad": _names(k.already_died), "lo": k.last_observed_round_stage, "lw": k.last_winner, "ls": k.loss_streak,
			"sw": k.has_swapped_teams, "cp": m.control_points.find(k.capture_point) if k.capture_point != null else -1,
			"ct": _js(k.client_tick(null))}
	if m is DuelMode:
		var dm: DuelMode = m
		var rooms := []
		for r in dm.rooms:
			rooms.append({"c": _names(r.controllers), "g": _rg(r.game), "m1": r.team1_mmr, "m2": r.team2_mmr,
				"st": r.state, "mid": r.match_id})
		var mt := {}
		for k in dm.matches:
			var mem := []
			for x in dm.matches[k].members:
				mem.append([x.entity, x.team_id])
			mt[k] = mem
		st["du"] = {"rooms": rooms, "un": _names(dm.unhandled), "tumc": dm.time_until_map_change,
			"ja": dm.join_allowed, "matches": mt}
	if m is FlMode:
		st["fl"] = {"dc": (m as FlMode).ticket_drain_counter}
	var cps := []
	for cp in m.control_points:
		cps.append({"name": cp.name, "p": cp.capture_progress, "rp": cp.replicated_capture_progress,
			"ow": cp.owning_team, "ca": cp.capturing_team, "u": cp.unchanged_capture_progress_time,
			"nuf": cp.net_update_frequency, "cap": cp.b_is_capturable, "t1": cp.b_team1_owns_prerequisites,
			"t2": cp.b_team2_owns_prerequisites, "sdis": cp.b_spawns_disabled, "stm": cp.spawns_team,
			"p1": cp.team1_presence, "p2": cp.team2_presence, "fl": cp.b_is_flashing, "lk": cp.locked,
			"ov": _names(cp.overlapping), "oc": _names(cp.overlaps_cache)})
	st["cps"] = cps
	if not m.control_points.is_empty():
		var dis := []
		for ps in m.d.starts:
			dis.append(ps.b_is_spawn_disabled)
		st["sdis"] = dis
	return st

# ---- host actions (each recorded in `ops`) ------------------------------------------------------------------------
func _op(o: Dictionary) -> void:
	ops.append(o)

func login(nm: String, bot: bool, sid := -1) -> void:
	var c := MordhauGameMode.Ctrl.new(nm, bot)
	m.post_login(c, sid)
	_op({"op": "login", "name": nm, "bot": bot, "sid": sid})

func logout(nm: String) -> void:
	var c := m.by_name(nm)
	if c == null:
		return
	m.logout(c)
	_op({"op": "logout", "name": nm})

func kill(killer: String, killed: String, damage_type := 0, weapon := "", kick := false) -> void:
	m.on_killed(m.by_name(killer) if killer != "" else null, m.by_name(killed), damage_type, weapon, kick)
	_op({"op": "kill", "killer": killer, "killed": killed, "dt": damage_type, "weapon": weapon, "kick": kick})

func despawn(nm: String) -> void:
	var c := m.by_name(nm)
	if c == null:
		return
	c.has_pawn = false
	_op({"op": "despawn", "name": nm})

func damage(victim: String, attacker: String, amount: float) -> void:
	m.on_damage(m.by_name(victim), m.by_name(attacker), amount)
	_op({"op": "damage", "victim": victim, "attacker": attacker, "amount": amount})

func set_loc(nm: String, p: Vector3) -> void:
	m.by_name(nm).location = p
	_op({"op": "loc", "name": nm, "p": [p.x, p.y, p.z]})

func overlap(cp_i: int, nm: String) -> void:
	m.control_points[cp_i].begin_overlap(m.by_name(nm))
	_op({"op": "overlap", "cp": cp_i, "name": nm})

func unoverlap(cp_i: int, nm: String) -> void:
	m.control_points[cp_i].end_overlap(m.by_name(nm))
	_op({"op": "unoverlap", "cp": cp_i, "name": nm})

# ---- plans -------------------------------------------------------------------------------------------------------
func _plan(n: int) -> void:
	for nm in _despawn:
		despawn(nm)
	_despawn.clear()
	if n == 0:
		for p in s.get("players", []):
			login(String(p.name), bool(p.get("bot", false)), int(p.get("sid", -1)))
		if String(s.get("plan", "")) == "skm_cp":
			(m as SkmMode).set_capture_point(ControlPoint.new(ModeData.control_point(SKM_CP), m.d.kv))
			_op({"op": "skm_cp"})
	for e in s.get("at", []):			# scripted one-off actions {n, op, ...}
		if int(e.n) == n:
			match String(e.op):
				"login": login(String(e.name), bool(e.get("bot", false)), int(e.get("sid", -1)))
				"logout": logout(String(e.name))
				"kill": kill(String(e.get("killer", "")), String(e.killed), int(e.get("dt", 0)), String(e.get("weapon", "")), bool(e.get("kick", false)))
	var every := int(s.get("every", 10))
	if n == 0 or n % every != 0:
		return
	match String(s.get("plan", "")):
		"ffa": _plan_ffa()
		"tdm": _plan_tdm()
		"skm", "skm_cp": _plan_skm(n)
		"duel": _plan_duel()
		"fl": _plan_fl(n)

func _live() -> Array:
	return m.controllers.filter(func(c): return c.has_pawn and c.alive)

# FFA: a kill every `every` ticks among the live pawns; every 7th a suicide, every 11th without a killer controller,
# every 3rd preceded by damage from a third player (assists: AssistScoreFactor 0 in FFA), every 5th a fall / kick flag
func _plan_ffa() -> void:
	var live := _live()
	if live.size() < 2:
		return
	_k += 1
	var killed: MordhauGameMode.Ctrl = live[(_k * 3 + 1) % live.size()]
	var killer: MordhauGameMode.Ctrl = live[_k % live.size()]
	var fav := m.by_name(String(s.get("favor", "")))
	if fav != null and live.has(fav) and _k % 4 != 0:
		killer = fav				# "favor": this player gets most kills (reaches ScoreToWin)
		if killed == fav:
			killed = live[(_k + 1) % live.size()] if live[(_k + 1) % live.size()] != fav else live[(_k + 2) % live.size()]
	if killer == killed:
		killer = live[(_k + 1) % live.size()]
	if live.size() >= 3 and _k % 3 == 0:
		var third: MordhauGameMode.Ctrl = live[(_k + 2) % live.size()]
		if third != killed:
			damage(killed.name, third.name, 30.0 + float(_k % 50))
	var kn := killer.name
	if _k % 7 == 0:
		kn = killed.name
	elif _k % 11 == 0:
		kn = ""
	var dmg_type := 3 if _k % 5 == 0 else 0
	kill(kn, killed.name, dmg_type, "Longsword", _k % 13 == 0)
	_despawn.append(killed.name)
	# the host moves pawns: spread the live ones so spawn preference sees distances
	for i in live.size():
		var c: MordhauGameMode.Ctrl = live[i]
		if c != killed:
			set_loc(c.name, c.location + Vector3(0.5 * float((_k + i) % 5 - 2), 0.0, 0.25 * float(i)))

# TDM: kills across teams, with assists (damage from up to two team mates first) and every 6th a team kill
func _plan_tdm() -> void:
	var live := _live()
	var t0 := live.filter(func(c): return c.team == 0)
	var t1 := live.filter(func(c): return c.team == 1)
	if t0.is_empty() or t1.is_empty():
		return
	_k += 1
	var att: Array = t0 if _k % 2 == 0 else t1
	var vic: Array = t1 if _k % 2 == 0 else t0
	var killer: MordhauGameMode.Ctrl = att[_k % att.size()]
	var killed: MordhauGameMode.Ctrl = vic[(_k / 2) % vic.size()]
	if _k % 6 == 0 and vic.size() >= 2:
		killer = vic[((_k / 2) + 1) % vic.size()]		# team kill
	for i in mini(att.size(), 2):
		var a: MordhauGameMode.Ctrl = att[(_k + i) % att.size()]
		damage(killed.name, a.name, 20.0 + 35.0 * float(i) + float(_k % 17))
	kill(killer.name, killed.name, 0, "Longsword", false)
	_despawn.append(killed.name)

# SKM: in RoundPlay, a living member of the round's losing team dies every `every` ticks (alternating which team
# loses by round); skm_cp: once the point unlocks, the round's winners stand on it instead of killing
func _plan_skm(n: int) -> void:
	var k: SkmMode = m
	if k.stage() != k.sd.stage.RoundPlay:
		return
	var rnd := int(k.team_scores[0] + k.team_scores[1])
	var lose := rnd % 2 if rnd < 4 else (rnd / 2) % 2
	var live := _live()
	var victims := live.filter(func(c): return c.team == lose)
	var winners := live.filter(func(c): return c.team != lose and c.team >= 0)
	if String(s.plan) == "skm_cp" and k.capture_point != null and not k.capture_point.locked:
		var cp_i: int = m.control_points.find(k.capture_point)
		for c in winners:
			if not k.capture_point.overlapping.has(c):
				overlap(cp_i, c.name)
		if rnd % 3 != 2:
			return
	elif String(s.plan) == "skm_cp" and k.capture_point != null and rnd % 3 != 2:
		return					# wait for the point to unlock (TimeToUnlock) and capture it
	if victims.is_empty() or winners.is_empty():
		return
	_k += 1
	var killed: MordhauGameMode.Ctrl = victims[0]
	if _k % 4 != 3:
		damage(killed.name, winners[_k % winners.size()].name, 40.0)
	kill(winners[0].name, killed.name, 0, "Longsword", false)
	_despawn.append(killed.name)

# DU / TF: in RoundPlay, a living member of the round's losing team (by room) dies; the dead pawn stays (rooms keep
# dead pawns until RestartRoom)
func _plan_duel() -> void:
	var dm: DuelMode = m
	for i in dm.rooms.size():
		var r: Dictionary = dm.rooms[i]
		if int(r.game.round.stage) != int(dm.dd.stage.RoundPlay):
			continue
		var rnd := int(r.game.team1_wins) + int(r.game.team2_wins)
		var lose := (rnd / 2) % 2 if rnd % 3 != 1 else 1 - (rnd / 2) % 2
		var victims: Array = r.controllers.filter(func(c): return c.team == lose and c.alive)
		var winners: Array = r.controllers.filter(func(c): return c.team != lose and c.alive)
		if victims.is_empty() or winners.is_empty():
			continue
		_k += 1
		var killed: MordhauGameMode.Ctrl = victims[_k % victims.size()]
		damage(killed.name, winners[0].name, 55.0)
		kill(winners[_k % winners.size()].name, killed.name, 0, "Longsword", false)

# FL: team 0 takes the next point it can capture (prerequisites owned, not hidden, not its own), standing on it until
# it is owned; one team-1 player contests the first point for a while, then dies; team 1 loses a player every
# `kill_every` plan steps (death tickets)
func _plan_fl(n: int) -> void:
	var fm: FlMode = m
	_k += 1
	var live := _live()
	var t0 := live.filter(func(c): return c.team == 0)
	var t1 := live.filter(func(c): return c.team == 1)
	var target := -1
	for i in fm.control_points.size():
		var cp: ControlPoint = fm.control_points[i]
		if cp.d.b_is_hidden_point or cp.owning_team == 0 or not cp.can_capture(0):
			continue
		target = i
		break
	for c in t0:
		var cur: int = fm.control_points.find(c.capture_point) if c.capture_point != null else -1
		if cur != target:
			if cur >= 0:
				unoverlap(cur, c.name)
			if target >= 0:
				overlap(target, c.name)
	if target >= 0 and not t1.is_empty():
		var d: MordhauGameMode.Ctrl = t1[0]
		var cur: int = fm.control_points.find(d.capture_point) if d.capture_point != null else -1
		if _k < int(s.get("contest", 6)):
			if cur != target:
				if cur >= 0:
					unoverlap(cur, d.name)
				overlap(target, d.name)
		elif cur >= 0:
			unoverlap(cur, d.name)
	if _k % int(s.get("kill_every", 9)) == 0 and not t1.is_empty() and not t0.is_empty():
		var killed: MordhauGameMode.Ctrl = t1[_k % t1.size()]
		var cur: int = fm.control_points.find(killed.capture_point) if killed.capture_point != null else -1
		if cur >= 0:
			unoverlap(cur, killed.name)
		kill(t0[0].name, killed.name, 0, "Longsword", false)
		_despawn.append(killed.name)

# ---- bots --------------------------------------------------------------------------------------------------------
# MordhauBotController with every fighter-input request logged with the requester's view right after it
class GoldenBot extends MordhauBotController:
	var req: Array = []
	var viewer: Callable
	func _init(b: BotBody, all_bodies: Array, prof: BotBehaviorProfile, shared_rng: UeRand, bt: BtTree) -> void:
		super(b, all_bodies, prof, shared_rng, bt)
	func request_attack(move: int, angle: float) -> void:
		super(move, angle)
		req.append({"bot": body.name, "kind": "attack", "mv": move, "angle": angle, "view": viewer.call(body)})
	func request_parry(bt: int) -> void:
		super(bt)
		req.append({"bot": body.name, "kind": "parry", "bt": bt, "view": viewer.call(body)})
	func request_feint() -> void:
		super()
		req.append({"bot": body.name, "kind": "feint", "view": viewer.call(body)})

func _net_id(n) -> int:
	if n == null:
		return -1
	for i in _nets.size():
		if is_same(_nets[i], n):
			return i
	_nets.append(n)
	return _nets.size() - 1

func _view(b: BotBody) -> Dictionary:
	var sys: MotionSystem = b.sys
	var mo = sys.motion
	var v := {"kind": mo.kind() if mo != null else "", "id": mo.get_instance_id() if mo != null else 0,
		"net_id": _net_id(sys.net), "is_attack": mo is AttackMotion, "is_parry": mo is ParryMotion,
		"is_feinted": mo is FeintedMotion, "start_time": mo.start_time if mo != null else 0.0}
	if mo is AttackMotion:
		v.merge({"stage": mo.stage, "release_end": mo.release_end, "windup_end": mo.windup_end,
			"has_chambered": mo.b_has_chambered, "has_hit": mo.b_has_hit, "attack_type": mo.type, "mv": mo.move,
			"angle_target": mo.angle_target, "early_release_duration": mo.get_early_release_duration()})
	if mo is ParryMotion:
		v.merge({"riposte_window_start": mo.riposte_window_start, "riposte_window_base": mo.pd.riposte_window_base,
			"riposte_window_extra": mo.pd.non_held_parry_extension_and_riposte_window_extra})
	return {"motion": v, "stamina_byte": sys.stamina_byte(), "health_byte": sys.health_byte()}

static func _weapon(w: WeaponData) -> Dictionary:
	if w == null:
		return {}
	return {"id": w.id, "chain": _j(w.chain), "native_class": w.native_class, "b_can_attack": w.b_can_attack,
		"stab_damage0": float(w.stab.damage[0]), "strike_damage0": float(w.strike.damage[0])}

func _bb(v):
	if v is BotBody:
		return {"body": v.name}
	if v is bool:
		return {"bool": v}
	if v is float or v is int:
		return {"float": float(v)}
	if v is Vector3:
		return {"vec": [v.x, v.y, v.z]}
	return {"other": str(v)}

func _bot_out(c: GoldenBot, bodies: Array) -> Dictionary:
	var bb := {}
	for k in c.blackboard:
		bb[String(k)] = _bb(c.blackboard[k])
	var per := []
	for b in c.perceived:
		var i: Dictionary = c.perceived[b]
		per.append([b.name, i.sight, i.hearing, i.damage, i.team, i.update_time])
	var pev := []
	for e in c.perception_events:
		pev.append([e.kind == "started", e.who])
	c.perception_events.clear()
	var evs := []
	for e in c.events:
		evs.append([e.kind, int(e.get("id", e.get("slot", 0))), bool(e.get("forced", false)), float(e.t)])
	c.events.clear()
	var stack := []
	if c.tree != null:
		for fr in c.tree.stack:
			stack.append([fr.node.name, fr.idx])
	var mr = c.move_request
	return {"mr": c.motion_random, "fm": c.facing_mode, "fa": c.facing_actor.name if c.facing_actor != null else "",
		"fl": _j(c.facing_location), "fo": _j(c.facing_offset), "fp": c.facing_param, "fs": c.facing_since,
		"mv": [_j(mr.dest), mr.acceptance, mr.t] if not mr.is_empty() else [], "ps": c.path_status,
		"mp": c.move_pending, "lr": _j(c.last_request), "ev": evs, "bb": bb,
		"bt": [c.tree.active.name if c.tree != null and c.tree.active != null else "", stack,
			c.tree.pending if c.tree != null else false, c.tree.pending_result if c.tree != null else -1],
		"lce": c.last_closest_enemy.name if c.last_closest_enemy != null else "", "lcet": c.last_closest_enemy_changed_time,
		"rce": c.really_close_enemy.name if c.really_close_enemy != null else "", "sat": c.closest_enemy_saturated,
		"rf": c.random_float, "nrf": c.next_random_float_assignment, "per": per, "pev": pev,
		"prof": _obj(c.profile.p),
		"lfm": c.profile.last_footworking_enemy_motion.get_instance_id() if c.profile.last_footworking_enemy_motion != null else 0,
		"wc": c.body.wants_crouch, "ws": c.body.wants_sprint, "lvt": c.last_voice_time}

func _run_bots() -> void:
	var dt := float(s.get("dt", 1.0 / 60.0))
	var w := CombatState.new(dt)
	w.record_trace = false
	var d := BotDuel.new(w)
	d.team_mode = bool(s.get("team_mode", false))
	var tree := String(s.get("tree", MordhauBotController.DEFAULT_TREE))
	var bodies_hdr := []
	for f in s.fighters:
		w.add_fighter(String(f.name), String(f.get("weapon", LS)))
		var p: Array = f.loc
		var b := d.add_body(String(f.name), Vector3(float(p[0]), float(p[1]), float(p[2])), float(f.yaw),
			float(f.get("length", 8.0)), int(f.get("team", 0)))
	var bots_hdr := []
	for f in s.fighters:
		if not bool(f.get("bot", true)):
			continue
		var b: BotBody = null
		for x in d.bodies:
			if x.name == String(f.name):
				b = x
		var behavior := String(f.get("behavior", "BOTBEHAVIOR_Knight"))
		var prof := BotBehaviorProfile.new(BotData.profile(behavior))
		var td: BotData.TreeDef = BotData.tree_def(tree) if tree != "" else null
		bots_hdr.append({"body": b.name, "profile": _obj(prof.p), "tree": _obj(td) if td != null else null})
		# MordhauBotController.make + BotDuel.add_bot
		if b.sys != null:
			b.max_walk_speed = BotData.max_walk_speed()
		var c := GoldenBot.new(b, d.bodies, prof, d.rng, BtTree.new(td) if td != null else null)
		c.viewer = _view
		c.begin_play()
		c.combat = w
		c.world_state = d.world_state
		c.team_mode = d.team_mode
		d.bots.append(c)
		for o in d.bots:
			o.update_perception()
	for b in d.bodies:
		var inv := []
		for it in b.inventory:
			inv.append({"weapon": _weapon(it.get("weapon")), "is_fists": bool(it.get("is_fists", false)),
				"ranged": bool(it.get("ranged", false))})
		bodies_hdr.append({"name": b.name, "team": b.team, "location": _j(b.location), "yaw": b.yaw,
			"velocity": _j(b.velocity), "capsule_radius": b.capsule_radius, "capsule_half_height": b.capsule_half_height,
			"capsule_scale_min": b.capsule_scale_min, "max_walk_speed": b.max_walk_speed, "mesh_scale_x": b.mesh_scale_x,
			"weapon_length": b.weapon_length, "facing_bone_height": b.facing_bone_height, "eye_height": b.eye_height,
			"inventory": inv, "right_hand": b.right_hand, "weapon": _weapon(b.sys.weapon), "pawn": _view(b)})
	var outs0 := {}
	for c in d.bots:
		outs0[c.body.name] = _bot_out(c, d.bodies)
	g.write_row({"kind": "header", "bodies": bodies_hdr, "bots": bots_hdr, "consts": _consts(), "dt": dt,
		"team_mode": d.team_mode, "ticks": int(s.ticks), "rng": [d.rng.seed, d.rng.calls], "out0": outs0,
		"ws": _ws(d)})
	for n in int(s.ticks):
		w.step()
		var pre := {}
		for b in d.bodies:
			pre[b.name] = _view(b)
		var req := []
		for c in d.bots:
			c.tick(w.dt)
			req.append_array(c.req)
			c.req.clear()
		var outs := {}
		for c in d.bots:
			outs[c.body.name] = _bot_out(c, d.bodies)
		g.write_row({"n": n, "now": w.now, "pre": pre, "req": req, "out": outs, "rng": [d.rng.seed, d.rng.calls],
			"ws": _ws(d)})

# the shared VoiceOrEmote cooldowns (null = never: -INF, which JSON cannot hold)
static func _ws(d: BotDuel) -> Array:
	var out := []
	for k in ["last_voice", "last_emote"]:
		out.append(float(d.world_state[k]) if d.world_state.has(k) else null)
	return out

# ---- records for modes without a GDScript port (Horde, Battle Royale): read here through the same readers -------
const GMS := "Mordhau/Content/Mordhau/Blueprints/GameModes/"
const HRD_GM := GMS + "Horde/BP_HordeGameMode"
const HRD_GS := GMS + "Horde/BP_HordeGameState"
const HRD_META := GMS + "Horde/BP_HordeGameModeMetadata"
const HRD_PS := GMS + "Horde/BP_HordePlayerState"
const HRD_SQUADS := GMS + "Horde/Squads/"
const BR_GM := GMS + "BattleRoyale/BP_BattleRoyaleGameMode"
const BR_GS := GMS + "BattleRoyale/BP_BattleRoyaleGameState"
const BR_META := GMS + "BattleRoyale/BP_BattleRoyaleGameModeMetadata"

static func _ref(v) -> String:
	return UePkg.strip(String(v.ObjectPath)) if v is Dictionary and v.has("ObjectPath") and v.ObjectPath is String else ""

static func _field(d: Dictionary, prefix: String):
	for k in d:
		if String(k).begins_with(prefix + "_") or String(k) == prefix:
			return d[k]
	return null

func _run_data() -> void:
	var hrd_map := String(s.get("hrd_map", "Mordhau/Content/Mordhau/Maps/DuelCamp/HRD_Camp"))
	var br_map := String(s.get("br_map", "Mordhau/Content/Mordhau/Maps/FeitoriaMap/BR_Feitoria"))
	var hm := _obj(MordhauModeData.new(hrd_map, HRD_GM, HRD_GS, HRD_META))
	var bm := _obj(MordhauModeData.new(br_map, BR_GM, BR_GS, BR_META))
	g.write_row({"kind": "header", "hrd_mode": hm, "horde": _horde_data(), "br_mode": bm, "br": _br_data(),
		"consts": _consts()})

# BP_HordeGameMode class defaults (CDO over the AMordhauGameMode ctor), BP_HordePlayerState, the SquadInfo assets
# (USquadInfo ctor rva=0x166fb50 under each asset), the enemy classes' KillReward, FC_HordeDamageModifier sampled at
# integer player counts (UCurveFloat::GetFloatValue at float(team-0 count): ProgressWave@2286)
func _horde_data() -> Dictionary:
	var gm := CombatData.class_defaults(HRD_GM)
	var ps := CombatData.class_defaults(HRD_PS)
	var cfg := {}
	for k in ["WaveEnemySpawnOffset", "IsSquadSpawningEnabled", "MinPlayerDifficultyModifier", "MaxPlayerDifficultyModifier",
			"ConsoleDifficultyModifier", "MinPlayerDelayModifier", "MaxPlayerDelayModifier", "ConsoleDelayModifier",
			"MaxEnemiesInWave", "TeamKillCoinPunishment", "NewPlayerOnJoinGoldMultiplier", "MaxNewPlayerOnJoinGold",
			"DeadWaveCompletionMultiplier", "KillScoreChange", "Wave"]:
		cfg[k] = gm.get(k)
	cfg["Coins"] = ps.get("Coins")
	cfg["SkillPoints"] = ps.get("SkillPoints")
	var waves := []
	for w in gm.get("SquadWaves", []):
		var mand := []
		for m in _field(w, "MandatorySquads"):
			mand.append(_ref(m).get_file())
		waves.append({"completion_reward": _field(w, "CompletionReward"), "difficulty_pool": _field(w, "DifficultyPool"),
			"mandatory_squads": mand})
	var squads := []
	var nat := NativeCtor.defaults("USquadInfo")
	var names := DirAccess.get_files_at(UePkg.root() + "/json/" + HRD_SQUADS)
	var sorted_names := []
	for f in names:
		if String(f).ends_with(".json"):
			sorted_names.append(String(f).get_basename())
	sorted_names.sort()
	for n in sorted_names:
		var ex := UePkg.export_of(UePkg.load_pkg(HRD_SQUADS + n), "SquadInfo")
		var d: Dictionary = nat.duplicate()
		for k in ex.get("Properties", {}):
			d[k] = ex.Properties[k]
		var mem := []
		for kv in d.get("Members", []):
			mem.append([String(kv.Key), int(kv.Value)])
		squads.append({"name": n, "spawn_min_wave": d.get("SpawnMinWave"), "spawn_max_wave": d.get("SpawnMaxWave"),
			"max_squads_per_wave": d.get("MaxSquadsPerWave"), "difficulty": d.get("Difficulty"),
			"delay_before_spawn": d.get("DelayBeforeSpawn"), "spawn_order_weight": d.get("SpawnOrderWeight"),
			"add_to_squad_pool": d.get("bAddToSquadPool"), "squad_id": d.get("SquadID"), "members": mem})
	var enemies := []
	for kv in gm.get("EnemyDatabase", []):
		var v: Dictionary = kv.Value
		var cls := _ref(_field(v, "CharacterType"))
		var eq := []
		for x in _field(v, "Equipment"):
			eq.append(_ref(x))
		var eq2 := []
		for x in _field(v, "SecondaryEquipment"):
			eq2.append(_ref(x))
		var cd := CombatData.class_defaults(cls) if cls != "" else {}
		enemies.append({"key": String(kv.Key), "character_class": cls, "behavior": _ref(_field(v, "Behavior")),
			"kill_reward": cd.get("KillReward", 0), "customization_variants": _field(v, "CustomizationVariants").size(),
			"equipment": eq, "secondary_equipment": eq2})
	var curve := _ref(gm.get("ReceivedDamageByPlayerNum"))
	var dmg := []
	for i in CURVE_SAMPLES:
		dmg.append(CombatData.curve_value(curve, float(i)))
	return {"config": cfg, "squad_waves": waves, "squads": squads, "enemies": enemies, "damage_by_player_count": dmg,
		"damage_curve": curve, "extras": _horde_extras()}

const HRD := GMS + "Horde/"
const HRD_PC := HRD + "BP_HordePlayerController"
const HRD_CHESTS := ["BP_HordeChestBase", "BP_HordeChestTier1", "BP_HordeChestTier2", "BP_HordeChestTier3"]
const DI_GS := HRD + "DemonInvasion/Framework/BP_DemonHordeGamestate"
const WEARABLE_PICKUP := "Mordhau/Content/Mordhau/Blueprints/Interactables/WearablePickups/BP_WearablePickup"
const DI_STAGES := 16	# FC_DemonHordeJIPCoins sampled at (CurrentStage - 1) / StageCount for StageCount 1..16

# a TMap<UClass*, float> (BP_BattleRoyaleBaseItemSpawn ItemList & co.) in map order, each class with the casts
# SpawnContents makes on the spawned actor (MordhauEquipment / BP_WearablePickup_C)
static func _chance_list(v) -> Array:
	var out := []
	for kv in (v if v is Array else []):
		var cls := String(kv.Key).get_slice("'", 1).get_slice(".", 0)
		out.append({"class": cls.get_file(), "chance": float(kv.Value), "kind": _item_kind(cls)})
	return out

static func _item_kind(cls: String) -> String:
	if cls == "":
		return "other"
	if UePkg.chain(cls).has(WEARABLE_PICKUP):
		return "wearable"
	if CombatData.is_class_of(CombatData.native_class(UePkg.native_root(cls)), "AMordhauEquipment"):
		return "equipment"
	return "other"

# r3: the Horde actors' class defaults (graves, chests, purchasables), the buy menu (BP_HordeGameState and
# BP_DemonHordeGamestate BuyMenuEntries), the skill tree (BP_HordePlayerController SkillInfo / SkillPrerequisites,
# keys = E_HordeSkill enumerator names; mh-mode maps them to the byte values) and the Demon Invasion game state CDO
static func _horde_extras() -> Dictionary:
	var gr := CombatData.class_defaults(HRD + "BP_HordePlayerGrave")
	var grave := {"revive_period": gr.get("RevivePeriod"), "auto_revive_time": gr.get("AutoReviveTime"),
		"max_interaction_hold_time": gr.get("MaxInteractionHoldTime")}
	var chests := []
	for n in HRD_CHESTS:
		var c := CombatData.class_defaults(HRD + n)
		chests.append({"class": n, "cost": c.get("Cost", 0), "respawn_time": c.get("RespawnTime", 0.0),
			"item_list": _chance_list(c.get("ItemList", [])), "second_item_list": _chance_list(c.get("SecondItemList", [])),
			"third_item_list": _chance_list(c.get("ThirdItemList", [])),
			"original_second_item_list": _chance_list(c.get("OriginalSecondItemList", [])),
			"original_third_item_list": _chance_list(c.get("OriginalThirdItemList", []))})
	var purch := []
	var names := DirAccess.get_files_at(UePkg.root() + "/json/" + HRD)
	var sorted_names := []
	for f in names:
		var b := String(f).get_basename()
		if String(f).ends_with(".json") and (b.begins_with("BP_HordePurchasable") or b.begins_with("BP_HordePurchaseable")):
			sorted_names.append(b)
	sorted_names.sort()
	for n in sorted_names:
		var c := CombatData.class_defaults(HRD + n)
		purch.append({"class": n, "cost": c.get("Cost", 0), "purchasable_class": _ref(c.get("PurchasableClass")).get_file(),
			"is_ammo_restock": c.get("IsAmmoRestock", false), "ammo_restock_amount": c.get("AmmoRestockAmount", 0)})
	var pc := CombatData.class_defaults(HRD_PC)
	var skills := []
	for kv in pc.get("SkillInfo", []):
		var v: Dictionary = kv.Value
		skills.append({"skill": String(kv.Key).get_slice("::", 1), "name": String(_field(v, "Name").get("SourceString", "")),
			"ultimate": _field(v, "Ultimate"), "percent_a": _field(v, "PercentA"), "percent_b": _field(v, "PercentB"),
			"integer_a": _field(v, "IntegerA"), "time_a": _field(v, "TimeA")})
	var prereq := []
	# the CDO map from the extract JSON (the package readers return "__unsupported__" for enum-keyed maps)
	var pcj = JSON.parse_string(FileAccess.get_file_as_string(UePkg.root() + "/json/" + HRD_PC + ".json"))
	var pre_raw = []
	for e in (pcj if pcj is Array else []):
		if String(e.get("Name", "")).begins_with("Default__"):
			pre_raw = e.get("Properties", {}).get("SkillPrerequisites", [])
	for kv in _pairs(pre_raw):
		prereq.append([String(kv[0]).get_slice("::", 1), String(kv[1]).get_slice("::", 1)])
	var merchant := []
	for x in pc.get("MerchantPurchasables", []):
		merchant.append(_ref(x).get_file())
	var di := CombatData.class_defaults(DI_GS)
	var stages := []
	for kv in di.get("StageSettings", []):
		var v: Dictionary = kv.Value
		var en := []
		for e in _field(v, "HordeEnemies"):
			en.append([String(e.Key), float(e.Value)])
		stages.append({"stage": int(kv.Key), "max_total_enemies": _field(v, "MaxTotalEnemies"),
			"wave_spawn_interval": _field(v, "WaveSpawnInterval"), "stage_spawn_delay": _field(v, "StageSpawnDelay"),
			"skill_points_granted": _field(v, "SkillPointsGranted"), "objectives": _field(v, "ActorArray").size(),
			"horde_enemies": en})
	var dscale := _ref(di.get("DifficultyScaling"))
	var dsamples := []
	for i in CURVE_SAMPLES:
		dsamples.append(CombatData.curve_value(dscale, float(i)) if dscale != "" else 0.0)
	var jip := "Mordhau/Content/Mordhau/Blueprints/GameModes/Horde/DemonInvasion/Data/FC_DemonHordeJIPCoins"
	var jips := []
	for n in range(1, DI_STAGES + 1):
		var row := []
		for st in range(0, n + 2):	# CurrentStage 0..n+1 (byte: stage 0 -> 255 / n)
			row.append(CombatData.curve_value(jip, float((st - 1) & 0xff) / float(n)))
		jips.append(row)
	return {"grave": grave, "chests": chests, "purchasables": purch,
		"buy_menu": _buy_menu(CombatData.class_defaults(HRD_GS)), "di_buy_menu": _buy_menu(di), "skills": skills,
		"skill_prerequisites": prereq, "merchant_purchasables": merchant,
		"demon": {"stages": stages, "stage_activation_delay": di.get("StageActivationDelay"),
			"difficulty_scaling": dsamples, "jip_coins": jips}}

# a TMap property as [key, value] pairs, whether the reader kept the [{Key, Value}] array or made a Dictionary
static func _pairs(v) -> Array:
	var out := []
	if v is Dictionary:
		for k in v:
			out.append([k, v[k]])
	elif v is Array:
		for kv in v:
			if kv is Dictionary and kv.has("Key"):
				out.append([kv.Key, kv.Value])
	return out

static func _buy_menu(gs: Dictionary) -> Array:
	var out := []
	for kv in gs.get("BuyMenuEntries", []):
		var v: Dictionary = kv.Value
		var cls := _ref(_field(v, "PurchasedActor"))
		out.append({"id": int(kv.Key), "purchased_actor": cls.get_file(), "kind": _item_kind(cls),
			"base_cost": _field(v, "BaseCost"), "is_ammo_restock": _field(v, "IsAmmoRestock"),
			"ammo_restock_amount": _field(v, "AmmoRestockAmount")})
	return out

# BP_BattleRoyaleGameState RoundStartDuration (the Blueprint's own variable)
func _br_data() -> Dictionary:
	return {"round_start_duration": ModeData.cdo_float(BR_GS, "RoundStartDuration")}

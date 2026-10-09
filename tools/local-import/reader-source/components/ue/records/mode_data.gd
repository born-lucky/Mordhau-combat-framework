# mode_data.gd - the game-mode and UI code's (game/mode/*, game/ui/*) data adapter: Blueprint packages (exports, CDO,
# class defaults down the Super chain, UserDefinedEnums), FText values and the map package's placed actors, so
# game/mode and game/ui never call the extract readers (UePkg) themselves (tests/test_boundary.gd). Widget packages
# are read by Umg (components/ue/records/umg.gd). game/ reads the typed records at the end of this file (GameModeDef,
# GameStateDef, Metadata, player start teams), never UE property names.
class_name ModeData

# every export of a package, as CUE4Parse wrote them
static func exports(path: String) -> Array:
	return UePkg.load_pkg(path)

# class default object (Default__X_C properties) of a Blueprint package
static func cdo(path: String) -> Dictionary:
	return UePkg.cdo(UePkg.load_pkg(path))

# merged class defaults down the Blueprint Super chain (UePkg.defaults)
static func class_defaults(path: String) -> Dictionary:
	return UePkg.defaults(path)

# the first export of `type` in a package ({} if none)
static func export_of(path: String, type: String) -> Dictionary:
	return UePkg.export_of(UePkg.load_pkg(path), type)

# FText as mdx json writes it: {CultureInvariantString} or {LocalizedString / SourceString}; "" for none
static func text(v) -> String:
	if v is Dictionary and v.has("CultureInvariantString"):
		return String(v.CultureInvariantString)
	return UePkg.text(v)

# UserDefinedEnum: "E_X::NewEnumeratorN": value, DisplayNameMap NewEnumeratorN -> display name; {display name: value}
static func user_enum(path: String) -> Dictionary:
	var e := export_of(path, "UserDefinedEnum")
	var disp := {}
	for kv in e.get("Properties", {}).get("DisplayNameMap", []):
		disp[String(kv.Key)] = text(kv.Value)
	var out := {}
	var names: Dictionary = e.get("Names", {})
	for n in names:
		var short := String(n).get_slice("::", 1)
		if disp.has(short):
			out[disp[short]] = int(names[n])
	return out

# placed actors of `type` in a map package: {export name: Properties}
static func actors_of(map_path: String, type: String) -> Dictionary:
	var out := {}
	for e in UePkg.load_pkg(map_path):
		if String(e.get("Type", "")) == type:
			out[String(e.Name)] = e.get("Properties", {})
	return out

# ---- typed records -------------------------------------------------------------------------------------------------
# AMordhauGameMode (+ Blueprint variables) class defaults: CombatData.class_defaults = NativeCtor (AMordhauGameMode
# ctor rva=0x157a930, zero-filled first) with the Blueprint chain over it. Blueprint-declared variables (RoundsToWin,
# MaxPeoplePerRoom, TimeToWaitForPlayers, TimeUntilMapChange) are not in the native layout; their CDO serializes them
# (no archetype holds them), so all reads are required.
class GameModeDef:
	var rounds_to_win := 0
	var max_people_per_room := 0
	var time_to_wait_for_players := 0.0
	var time_until_map_change := 0.0
	var map_prefixes := {}				# MapPrefixes: prefix -> game mode class package

static func game_mode(path: String) -> GameModeDef:
	var r := UeRec.new(CombatData.class_defaults(path), path)
	var g := GameModeDef.new()
	g.rounds_to_win = r.i("RoundsToWin")
	g.max_people_per_room = r.i("MaxPeoplePerRoom")
	g.time_to_wait_for_players = r.f("TimeToWaitForPlayers")
	g.time_until_map_change = r.f("TimeUntilMapChange")
	for e in r.arr("MapPrefixes"):
		var er := UeRec.new(e, path + ".MapPrefixes[]", r.errors)
		var gm := er.sub("GameMode")
		var ap := gm.s("AssetPathName")		# FSoftClassPath "/Game/...X.X_C"
		g.map_prefixes[er.s("Name")] = ap
	return r.done(g)

# AMordhauGameState class defaults (native ctor rva=0x157e160 + Blueprint chain). TeamColors (TArray<FLinearColor>) is
# not decoded by NativeCtor; BP_MordhauGameState serializes it.
class GameStateDef:
	var default_warmup_time := 0.0
	var b_allow_health_regen := false
	var team_colors: Array[Color] = []	# linear

static func game_state(path: String) -> GameStateDef:
	var r := UeRec.new(CombatData.class_defaults(path), path)
	var g := GameStateDef.new()
	g.default_warmup_time = r.f("DefaultWarmupTime")
	g.b_allow_health_regen = r.b("bAllowHealthRegen")
	for color in r.arr("TeamColors"):
		var cr := UeRec.new(color, path + ".TeamColors[]", r.errors)
		g.team_colors.append(Color(cr.f("R"), cr.f("G"), cr.f("B"), cr.f("A")))
	return r.done(g)

# UGameModeMetadata / UMapMetadata CDO: Name / Description FText, Prefix FString (absent = empty: FText / FString
# default-construct empty)
class Metadata:
	var name := ""
	var prefix := ""
	var description := ""

static func metadata(path: String) -> Metadata:
	var r := UeRec.new(cdo(path), path)
	var m := Metadata.new()
	m.name = r.text_or("Name")
	m.prefix = r.s_or("Prefix", "")
	m.description = r.text_or("Description")
	return r.done(m)

# placed AMordhauPlayerStart actors of a map: {export name: Team}. A placed actor holds only its delta; absent Team =
# the AMordhauPlayerStart ctor value (NativeCtor, merged under it)
static func player_start_teams(map_path: String) -> Dictionary:
	var out := {}
	var nat := NativeCtor.defaults("AMordhauPlayerStart")
	var props := actors_of(map_path, "MordhauPlayerStart")
	for nm in props:
		var d: Dictionary = nat.duplicate()
		for k in props[nm]:
			d[k] = props[nm][k]
		var r := UeRec.new(d, "%s.%s" % [map_path.get_file(), nm])
		var team := r.i("Team")
		if r.done(true) != null:
			out[nm] = team
	return out

# a Blueprint variable of a CDO that the Blueprint declares itself (its CDO serializes it; required)
static func cdo_float(path: String, key: String) -> float:
	var r := UeRec.new(cdo(path), path)
	var v := r.f(key)
	return v if r.done(true) != null else NAN

static func cdo_int(path: String, key: String) -> int:
	var r := UeRec.new(cdo(path), path)
	var v := r.i(key)
	return v if r.done(true) != null else 0

# a bool Blueprint variable whose CDO may leave it out (a Blueprint bool absent from the CDO delta is false)
static func cdo_bool_or(path: String, key: String, def := false) -> bool:
	var r := UeRec.new(cdo(path), path)
	return r.b_or(key, def)

# a float array Blueprint variable of a CDO (required; e.g. BP_SkirmishGameMode RoundPointsPerTeam [0, 0])
static func cdo_floats(path: String, key: String) -> Array:
	var r := UeRec.new(cdo(path), path)
	var out := []
	for v in r.arr(key):
		out.append(float(v))
	r.done(true)
	return out

# ---- Free-For-All (BP_DeathmatchGameMode / BP_DeathmatchGameState) ------------------------------------------------
# AMordhauGameMode scoring / respawn fields (native ctor rva=0x157a930 replayed by NativeCtor, Blueprint chain over it:
# BP_MordhauGameMode KillScoreChange 100 / TeamKillScoreChange -100 over the ctor's 10 / -10, BP_DeathmatchGameMode
# AssistScoreFactor 0). All are stored by the ctor or a CDO, so every read is required; bPlayersSpawnInWaves is not
# stored by the ctor (zero-filled by StaticAllocateObject before construction: false).
class ScoringDef:
	var kill_score_change := 0.0
	var team_kill_score_change := 0.0
	var kill_team_score_change := 0.0
	var team_kill_team_score_change := 0.0
	var player_respawn_time := 0.0
	var b_players_spawn_in_waves := false
	var b_suicide_decrements_kills := false
	var b_team_kills_decrement_killer_kills := false
	var b_team_kills_increment_killed_deaths := false
	var assist_score_factor := 0.0
	var assist_damage_to_count_as_kill := 0
	var b_is_scoring_disabled := true		# ctor true; HandleMatchHasStarted 0x1599ee0 clears it
	var spawn_protection_duration := 0.0

static func scoring(path: String) -> ScoringDef:
	var r := UeRec.new(CombatData.class_defaults(path), path)
	var g := ScoringDef.new()
	g.kill_score_change = r.f("KillScoreChange")
	g.team_kill_score_change = r.f("TeamKillScoreChange")
	g.kill_team_score_change = r.f("KillTeamScoreChange")
	g.team_kill_team_score_change = r.f("TeamKillTeamScoreChange")
	g.player_respawn_time = r.f("PlayerRespawnTime")
	g.b_players_spawn_in_waves = r.b_or("bPlayersSpawnInWaves", false)
	g.b_suicide_decrements_kills = r.b("bSuicideDecrementsKills")
	g.b_team_kills_decrement_killer_kills = r.b("bTeamKillsDecrementKillerKills")
	g.b_team_kills_increment_killed_deaths = r.b("bTeamKillsIncrementKilledDeaths")
	g.assist_score_factor = r.f("AssistScoreFactor")
	g.assist_damage_to_count_as_kill = r.i("AssistDamageToCountAsKill")
	g.b_is_scoring_disabled = r.b("bIsScoringDisabled")
	g.spawn_protection_duration = r.f("SpawnProtectionDuration")
	return r.done(g)

# BP_DeathmatchGameMode's own variable ScoreToWin (3000; the Blueprint declares it, its CDO serializes it)
static func score_to_win(path: String) -> float:
	return cdo_float(path, "ScoreToWin")

# AMordhauGameState match fields (ctor rva=0x157e160 + Blueprint chain): MatchDurationMax (int, s; not stored by the
# ctor: zero-filled 0 = no limit unless a Blueprint CDO sets it, e.g. BP_DeathmatchGameState 1200), TeamCount (byte),
# bIsTeamMode, WarmupEnd (ctor -1; AMordhauGameState::BeginPlay 0x1584900 stores -1 again on the server, disasm
# 0x141584f18 `mov dword ptr [rsi + 0x690], 0xbf800000`), DefaultWarmupTime, bAllowSpawning (ctor true; BP_DuelGameState
# and BP_Group3v3GameState false; read by AMordhauGameMode::ControllerCanRestart_Implementation 0x1587e50 at 0x141587f2d
# `cmp byte ptr [rbx + 0x696], 0`, 0x696 = bAllowSpawning in extract/native/types/AMordhauGameState.h)
class MatchStateDef:
	var match_duration_max := 0
	var team_count := 0
	var b_is_team_mode := false
	var warmup_end := 0.0
	var default_warmup_time := 0.0
	var team_colors: Array[Color] = []
	var team_names: PackedStringArray = []	# TeamNames (FText), BP_MordhauGameState
	var b_uses_auto_assign := false			# bUsesAutoAssign (ctor; BP_TeamDeathmatchGameState true)
	var b_allow_spawning := true			# bAllowSpawning (ctor true)

static func match_state(path: String) -> MatchStateDef:
	var r := UeRec.new(CombatData.class_defaults(path), path)
	var g := MatchStateDef.new()
	g.match_duration_max = r.i_or("MatchDurationMax", 0)		# zero-filled when no CDO stores it
	g.team_count = r.i("TeamCount")
	g.b_is_team_mode = r.b("bIsTeamMode")
	g.warmup_end = r.f("WarmupEnd")
	g.default_warmup_time = r.f("DefaultWarmupTime")
	for color in r.arr_or("TeamColors"):
		var cr := UeRec.new(color, path + ".TeamColors[]", r.errors)
		g.team_colors.append(Color(cr.f("R"), cr.f("G"), cr.f("B"), cr.f("A")))
	for n in r.arr_or("TeamNames"):
		g.team_names.append(text(n))
	g.b_uses_auto_assign = r.b_or("bUsesAutoAssign", false)		# not stored by the ctor: zero-filled false
	g.b_allow_spawning = r.b("bAllowSpawning")
	return r.done(g)

# UBotBehaviorProfile DefaultTeam (native ctor; no shipped behavior profile overrides it): the team a bot asks for when
# it has none (AMordhauAIController::Tick 0x15185b0 -> RequestedAssignTeam(DefaultTeam))
static func bot_default_team() -> int:
	var r := UeRec.new(NativeCtor.defaults("UBotBehaviorProfile"), "UBotBehaviorProfile")
	var v := r.i("DefaultTeam")
	return v if r.done(true) != null else -1

# a float Blueprint variable a game mode Blueprint declares (TeamScoreToWin, ScoreToWin)
static func mode_float(path: String, key: String) -> float:
	return cdo_float(path, key)

# AMordhauAIController bAutoRespawn (AMordhauAIController::Tick 0x15185b0: no pawn and bAutoRespawn -> bWantsRespawn)
static func ai_auto_respawn(path: String) -> bool:
	var r := UeRec.new(CombatData.class_defaults(path), path)
	var v := r.b("bAutoRespawn")
	return v if r.done(true) != null else false

# Placed AMordhauPlayerStart actors (native class or BP_MordhauPlayerStart_C) of a map and its streamed sub-levels:
# name, Team (ctor -2 = any team, AMordhauPlayerStart ctor 0x15b5450), bIsSpawnDisabled (not stored by the ctor:
# false) and the world transform of the actor's root component (UeLevel.world_xf, Godot space, metres).
class PlayerStartDef:
	var name := ""
	var team := -2
	var b_is_spawn_disabled := false
	var xf := Transform3D.IDENTITY

static func player_starts(map_path: String) -> Array:
	var out := []
	var nat := NativeCtor.defaults("AMordhauPlayerStart")
	for lv in UeLevel.levels(map_path):
		for e in UePkg.load_pkg(lv[0]):
			var t := String(e.get("Type", ""))
			if t != "MordhauPlayerStart" and not (t.ends_with("_C") and t.contains("PlayerStart")):
				continue
			var d: Dictionary = nat.duplicate()
			var own: Dictionary = e.get("Properties", {})
			for k in own:
				d[k] = own[k]
			var r := UeRec.new(d, "%s.%s" % [String(lv[0]).get_file(), e.Name])
			var ps := PlayerStartDef.new()
			ps.name = String(e.Name)
			ps.team = r.i("Team")
			ps.b_is_spawn_disabled = r.b_or("bIsSpawnDisabled", false)
			var root := UeLevel.obj(own.get("RootComponent"))
			ps.xf = (lv[1] as Transform3D) * UeLevel.world_xf(root) if not root.is_empty() else lv[1]
			if r.done(true) != null:
				out.append(ps)
	return out

# map metadata: the maps a UMapMetadata lists (Maps: soft paths "/Game/.../X.X") as package names "X"
static func metadata_maps(path: String) -> PackedStringArray:
	var r := UeRec.new(cdo(path), path)
	var out := PackedStringArray()
	for m in r.arr_or("Maps"):
		var mr := UeRec.new(m, path + ".Maps[]", r.errors)
		out.append(mr.s("AssetPathName").get_file().get_basename())
	r.done(true)
	return out

# UMordhauSingleton BotCharacterProfiles display names, in array order (FCharacterProfile Name FText)
static func bot_profile_names() -> PackedStringArray:
	var out := PackedStringArray()
	for k in UeWearable.profiles("BotCharacterProfiles"):
		var v = JSON.parse_string(String(k)) if String(k).begins_with("{") else null
		out.append(text(v) if v is Dictionary else String(k))
	return out

# AMordhauEquipment EquipmentName (FText) of an equipment Blueprint: what the kill feed prints (ReceiveKillNotify)
static func equipment_name(path: String) -> String:
	if path == "":
		return ""
	var r := UeRec.new(CombatData.class_defaults(path), path)
	var v := r.text_or("EquipmentName")
	r.done(true)
	return v

# ---- capture points (AControlPoint; hook added by the modes builder r6) ------------------------------------------
# AControlPoint class defaults: NativeCtor (AControlPoint ctor rva=0x14e30e0: OwningTeam / CapturingTeam 255,
# UncaptureSpeed 0.5, NetworkSmoothTime 0.1, bPreventSpawningIfContested, AwardScore* 10 / 50 / 2 s, bIsCapturable)
# with the Blueprint chain over it (BP_CapturePoint, BP_SkirmishCapturePoint: AwardScore* 20 / 150 / 1.5 s, curves,
# UncaptureSpeed; BP_SkirmishCapturePoint TimeToUnlock 30 / Locked / bShouldPauseCaptureIfEnemyNear). A placed actor's
# own properties (OwningTeam, prerequisites, SpawnPoints, ObjectiveWinDelay) are merged over its class defaults.
# Blueprint-only variables absent from a class (TimeToUnlock, Locked, ObjectiveWinDelay) read as -1 / false / 0.
class ControlPointDef:
	var name := ""						# placed actor export name ("" for a class record)
	var cls := ""						# Blueprint package of the actor's class
	var display_name := ""				# Name (FText)
	var owning_team := 0				# read: OwningTeam (always set by _cp_record)
	var capturing_team := 0			# read: CapturingTeam
	var award_score_interval := 0.0
	var award_score_capturing := 0
	var award_score_captured := 0
	var award_score_neutralizing := 0
	var award_score_neutralized := 0
	var uncapture_speed := 0.0
	var network_smooth_time := 0.0
	var b_prevent_spawning_if_contested := false
	var b_is_capturable := false
	var b_should_pause_capture_if_enemy_near := false
	var b_is_hidden_point := false
	var capture_speed_curve := ""		# UCurveFloat package ("" = none)
	var neutralize_speed_curve := ""
	var time_to_unlock := -1.0			# BP_SkirmishCapturePoint TimeToUnlock (-1: not a variable of this class)
	var locked := false					# BP_SkirmishCapturePoint Locked
	var objective_win_delay := 0.0		# BP_CapturePoint ObjectiveWinDelay
	var team1_prerequisites := PackedStringArray()	# placed: Team1PrerequisitePoints (actor names)
	var team2_prerequisites := PackedStringArray()
	var spawn_points := PackedStringArray()			# placed: SpawnPoints (AMordhauPlayerStart names)

static func _cp_names(r: UeRec, k: String) -> PackedStringArray:
	var out := PackedStringArray()
	for v in r.arr_or(k):
		var on := String(v.get("ObjectName", "")) if v is Dictionary else ""
		out.append(on.get_slice("'", 1).get_slice(".", on.get_slice("'", 1).get_slice_count(".") - 1))
	return out

static func _cp_record(d: Dictionary, label: String) -> ControlPointDef:
	var r := UeRec.new(d, label)
	var c := ControlPointDef.new()
	c.display_name = r.text_or("Name")
	c.owning_team = r.i("OwningTeam")
	c.capturing_team = r.i("CapturingTeam")
	c.award_score_interval = r.f("AwardScoreInterval")
	c.award_score_capturing = r.i("AwardScoreCapturing")
	c.award_score_captured = r.i("AwardScoreCaptured")
	c.award_score_neutralizing = r.i("AwardScoreNeutralizing")
	c.award_score_neutralized = r.i("AwardScoreNeutralized")
	c.uncapture_speed = r.f("UncaptureSpeed")
	c.network_smooth_time = r.f("NetworkSmoothTime")
	c.b_prevent_spawning_if_contested = r.b("bPreventSpawningIfContested")
	c.b_is_capturable = r.b("bIsCapturable")
	c.b_should_pause_capture_if_enemy_near = r.b_or("bShouldPauseCaptureIfEnemyNear", false)
	c.b_is_hidden_point = r.b_or("bIsHiddenPoint", false)
	c.capture_speed_curve = r.obj("CaptureSpeedCurve")
	c.neutralize_speed_curve = r.obj("NeutralizeSpeedCurve")
	c.time_to_unlock = r.f_or("TimeToUnlock", -1.0)
	c.locked = r.b_or("Locked", false)
	c.objective_win_delay = r.f_or("ObjectiveWinDelay", 0.0)
	c.team1_prerequisites = _cp_names(r, "Team1PrerequisitePoints")
	c.team2_prerequisites = _cp_names(r, "Team2PrerequisitePoints")
	c.spawn_points = _cp_names(r, "SpawnPoints")
	return r.done(c)

static func control_point(bp_path: String) -> ControlPointDef:
	var c := _cp_record(CombatData.class_defaults(bp_path), bp_path)
	if c != null:
		c.cls = UePkg.strip(bp_path)
	return c

# every placed AControlPoint subclass actor of a map package (and its streamed sub-levels), in export order
static var _cp_classes := {}

static func control_points(map_path: String) -> Array:
	var out := []
	for lv in UeLevel.levels(map_path):
		for e in UePkg.load_pkg(lv[0]):
			var cls := String(e.get("Class", ""))
			if not cls.begins_with("BlueprintGeneratedClass'"):
				continue
			var bp := UePkg.strip(cls.get_slice("'", 1).get_slice(".", 0))
			if not _cp_classes.has(bp):
				_cp_classes[bp] = UePkg.exists(bp) and CombatData.is_class_of(CombatData.native_class(UePkg.native_root(bp)), "AControlPoint")
			if not _cp_classes[bp]:
				continue
			var d: Dictionary = UePkg._merge(CombatData.class_defaults(bp), e.get("Properties", {}))
			var c := _cp_record(d, "%s.%s" % [String(lv[0]).get_file(), e.Name])
			if c != null:
				c.name = String(e.Name)
				c.cls = bp
				out.append(c)
	return out

# ---- navigation (hook added by the ai builder r9: game/actor/bot_world.gd) ----------------------------------------
# The Recast navmesh agent of DefaultEngine.ini [/Script/NavigationSystem.RecastNavMesh] (AgentRadius 50, AgentHeight
# 192, AgentMaxStepHeight 60, CellSize 5, CellHeight 5) with AgentMaxSlope from BaseEngine.ini (44; DefaultEngine.ini
# does not set it). UE units (cm, degrees).
class NavAgentDef:
	var radius := 0.0
	var height := 0.0
	var max_step := 0.0
	var max_slope := 0.0
	var cell_size := 0.0
	var cell_height := 0.0

static func nav_agent() -> NavAgentDef:
	const S := "/Script/NavigationSystem.RecastNavMesh"
	var a := NavAgentDef.new()
	a.radius = UeConfig.float_value("DefaultEngine.ini", S, "AgentRadius")
	a.height = UeConfig.float_value("DefaultEngine.ini", S, "AgentHeight")
	a.max_step = UeConfig.float_value("DefaultEngine.ini", S, "AgentMaxStepHeight")
	a.cell_size = UeConfig.float_value("DefaultEngine.ini", S, "CellSize")
	a.cell_height = UeConfig.float_value("DefaultEngine.ini", S, "CellHeight")
	var sl := UeConfig.value("BaseEngine.ini", S, "AgentMaxSlope").trim_suffix("f")		# "44.f": a C++ float literal
	a.max_slope = float(sl) if sl.is_valid_float() else NAN
	return a

# NavMeshBoundsVolume actors of a map and its streamed levels: Godot-space AABBs (metres) of the brush collision
# (BrushComponent BrushBodySetup AggGeom ConvexElems VertexData, tagged properties) through the BrushComponent's
# world transform (UeLevel.world_xf; the volume's RootComponent)
static func nav_bounds(map_path: String) -> Array:
	var out := []
	for lv in UeLevel.levels(map_path):
		for e in UePkg.load_pkg(lv[0]):
			if String(e.get("Type", "")) != "NavMeshBoundsVolume":
				continue
			var p: Dictionary = e.get("Properties", {})
			var root := UeLevel.obj(p.get("RootComponent"))
			var rr := UeRec.new(UeLevel.obj(UeRec.new(root.get("Properties", {}), "BrushComponent").d.get("BrushBodySetup")).get("Properties", {}),
				"%s.%s.BrushBodySetup" % [String(lv[0]).get_file(), e.Name])
			var pts := []
			for ce in rr.sub("AggGeom").arr("ConvexElems"):
				pts.append_array(UeRec.new(ce, rr.what + ".ConvexElems[]", rr.errors).arr("VertexData"))
			if rr.done(true) == null or root.is_empty() or pts.is_empty():
				push_error("ModeData.nav_bounds: %s %s has no brush collision" % [String(lv[0]).get_file(), e.Name])
				continue
			var xf: Transform3D = (lv[1] as Transform3D) * UeLevel.world_xf(root)
			var box := AABB(xf * (UeLevel.swap(pts[0]) * 0.01), Vector3.ZERO)
			for pt in pts:
				box = box.expand(xf * (UeLevel.swap(pt) * 0.01))
			out.append(box)
	return out

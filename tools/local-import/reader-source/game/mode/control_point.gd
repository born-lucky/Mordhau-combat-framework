# control_point.gd - a capture point: native AControlPoint (extract/native/decomp/AControlPoint.cpp) with the Blueprint
# subclasses' overrides the modes use (BP_CapturePoint / BP_TeamBaseCapturePoint for Frontline, BP_SkirmishCapturePoint
# for Skirmish). Engine-agnostic: the adapter (or a test) reports the pawns that begin / end overlapping the capture
# area; the game mode ticks every point once per frame (tick) and reads owning_team / capture_progress.
# Constants: the typed record ModeData.ControlPointDef (native ctor + Blueprint CDO chain + the placed actor).
#
# Native sources (Mordhau-Win64-Shipping.exe build 702625635, PDB names; offsets from extract/native/types/*.h):
#   ctor          AControlPoint::AControlPoint rva=0x14e30e0 (OwningTeam / CapturingTeam 255, SpawnsTeam -1,
#                 UnchangedCaptureProgressTime 999, LastSetUIProgress -1, NetUpdateFrequency 10): read through ModeData
#   BeginPlay     AControlPoint::BeginPlay rva=0x14f0030 (server: a point placed with an owner starts captured)
#   Tick          AControlPoint::Tick rva=0x1517270 (prerequisites, server progress / client smoothing, flashing)
#   progress      AControlPoint::UpdateCaptureProgress rva=0x151b3f0, AControlPoint::SetCaptureProgress rva=0x1515740
#   presence      AControlPoint::UpdatePresenceNumbers rva=0x151e920 (live characters, character Team byte 0 / 1),
#                 AControlPoint::OnCaptureAreaBeginOverlap rva=0x1507fa0 / OnCaptureAreaEndOverlap rva=0x1508000
#                 (AMordhauCharacter CurrentCapturePoint +0xd80 / CurrentCapturePointTime +0xd88)
#   spawns        AControlPoint::UpdateSpawns rva=0x151fb90 (AMordhauPlayerStart bIsSpawnDisabled +0x250, Team +0x254)
#   CanCapture    AControlPoint::CanCapture rva=0x14f2390
#   setters       AControlPoint::SetCapturingTeam rva=0x1515b30, AControlPoint::SetOwningTeam rva=0x1515c30,
#                 OnRep_CapturingTeam rva=0x150e4e0, OnRep_OwningTeam rva=0x150e540 (fire the *Changed event)
#   client        AControlPoint::OnRep_ReplicatedCaptureProgress rva=0x150ecf0; Tick's non-authority branch smooths
#                 CaptureProgress toward ReplicatedCaptureProgress / 255 over NetworkSmoothTime
# Blueprint (scripts/kismet bytecode; literals: ModeKismet "skm_cp_*"):
#   BP_SkirmishCapturePoint RoundStarted: SetCaptureProgress(0, 255, false) [5603]; Locked = true, OnRep_Locked
#     (bIsCapturable = !Locked) [276]; RetriggerableDelay(TimeToUnlock) [183] -> Locked = false, OnRep_Locked [104]
#   BP_SkirmishCapturePoint OnCapturingTeamChanged (local player): GetTimeSeconds > 3 and ShowAnnouncements and
#     CapturingTeam != 255: GetTeamRelevance (local team 0 / 1: Team != local team; else 2) 0 -> "We are capturing the
#     point!", 1 -> "Enemy is capturing the point!" (4 s) [307..1135]. Its OnOwningTeamChanged announcements cast the
#     HUD to BP_FrontlineHUD, which BP_SkirmishHUD (: BP_MordhauTeamHUD) is not: nothing shows in Skirmish.
# Not ported (UNCONFIRMED / out of scope): the capture area's geometry (the adapter decides overlaps); UpdateVisuals /
# banners / UI material / widgets (client visuals: events only); BP_CapturePoint objectives (destroyables, delivery,
# pushables: ObjectivesChanged / OnObjectivesCompleted); push mode; actor tick order relative to the game mode and the
# game state (the mode ticks points before its own Blueprint tick).
class_name ControlPoint
extends RefCounted

const NO_TEAM := 255

var d: ModeData.ControlPointDef
var name := ""
var authority := true					# Role == ROLE_Authority (3)
var dedicated := false					# a dedicated server skips the client visuals (flashing)
# replicated / server state
var capture_progress := 0.0				# CaptureProgress +0x2d0
var replicated_capture_progress := 0	# ReplicatedCaptureProgress +0x2d9 (byte)
var owning_team := NO_TEAM				# OwningTeam +0x2e0
var capturing_team := NO_TEAM			# CapturingTeam +0x2e1
var unchanged_capture_progress_time := 0.0	# UnchangedCaptureProgressTime +0x2d4
var net_update_frequency := 0.0
var b_is_capturable := true				# bIsCapturable +0x24b
var b_team1_owns_prerequisites := false	# +0x24c (not stored by the ctor: false until the first Tick)
var b_team2_owns_prerequisites := false	# +0x24d
var b_spawns_disabled := false			# +0x24e
var spawns_team := -1					# SpawnsTeam +0x258
var team1_presence := 0					# +0x318
var team2_presence := 0					# +0x31c
var b_is_flashing := false				# +0x320
var b_has_ever_replicated_progress := false
var team1_prerequisites: Array = []		# [ControlPoint] Team1PrerequisitePoints
var team2_prerequisites: Array = []
var spawn_points: Array = []			# [ModeData.PlayerStartDef] SpawnPoints
var overlapping: Array = []				# characters overlapping the capture area (MordhauGameMode.Ctrl with a pawn)
var overlaps_cache: Array = []			# OverlapsCache (UpdatePresenceNumbers)
# BP_SkirmishCapturePoint
var locked := false
var _unlock_at := -1.0
var events: Array = []					# {kind, ...}: team changes, prerequisites, scores, announcements
var add_score: Callable					# AMordhauPlayerState::AddScore (MordhauGameMode.add_score)
var _ks: Callable						# bytecode literal lookup (ModeKismet via the mode data)

func _init(def: ModeData.ControlPointDef, kv := Callable()) -> void:
	d = def
	name = def.name
	_ks = kv
	owning_team = def.owning_team
	capturing_team = def.capturing_team
	unchanged_capture_progress_time = UNCHANGED_AT_CTOR
	b_is_capturable = def.b_is_capturable
	locked = def.locked
	net_update_frequency = NET_UPDATE_FREQUENCY_ACTIVE

# AControlPoint ctor rva=0x14e30e0 stores UnchangedCaptureProgressTime 999 (0x4479c000) and NetUpdateFrequency 10;
# Tick rva=0x1517270 drops NetUpdateFrequency to 1 after 2 s unchanged (0x3f800000 / 0x40000000 immediates)
const UNCHANGED_AT_CTOR := 999.0
const NET_UPDATE_FREQUENCY_ACTIVE := 10.0
const NET_UPDATE_FREQUENCY_IDLE := 1.0
const NET_IDLE_AFTER_S := 2.0
# Tick rva=0x1517270 flashing threshold: bIsFlashing follows UnchangedCaptureProgressTime >= 0.5
const FLASH_AFTER_S := 0.5
# SetCaptureProgress rva=0x1515740: ReplicatedCaptureProgress = (byte)(int)(CaptureProgress * 255);
# Tick / OnRep_ReplicatedCaptureProgress rva=0x150ecf0: CaptureProgress = Replicated * 0.003921569 (1/255)
const REP_SCALE := 255.0
const REP_INV := 0.003921569
# Tick rva=0x1517270: client smoothing snaps when within 0.0001; unchanged when |delta| <= 1e-08
const SNAP_EPS := 0.0001
const UNCHANGED_EPS := 1e-08
# SetCaptureProgress rva=0x1515740: AwardScoreInterval is used only when |interval| > 1e-08
const INTERVAL_EPS := 1e-08
# ClientReceiveScoreNoState reasons SetCaptureProgress rva=0x1515740 passes: 8 capturing, 9 captured,
# 10 neutralizing, 11 neutralized (EScoreFeedReason values; names UNCONFIRMED)
const REASON_CAPTURING := 8
const REASON_CAPTURED := 9
const REASON_NEUTRALIZING := 10
const REASON_NEUTRALIZED := 11

func _ev(kind: String, extra := {}) -> void:
	var e := {"kind": kind, "point": name}
	e.merge(extra)
	events.append(e)

func drain() -> Array:
	var e := events
	events = []
	return e

# AControlPoint::BeginPlay rva=0x14f0030 (server): OwningTeam != 255 -> CapturingTeam = OwningTeam (event),
# SetCaptureProgress(1, OwningTeam, false); then UpdateSpawns
func begin_play() -> void:
	if authority:
		if owning_team != NO_TEAM:
			set_capturing_team(owning_team)
			set_capture_progress(1.0, owning_team, false, 0.0)
		update_spawns()

# AControlPoint::CanCapture rva=0x14f2390: team 0 -> bTeam1OwnsPrerequisites, team 1 -> bTeam2..., else false
func can_capture(team: int) -> bool:
	if team == 0:
		return b_team1_owns_prerequisites
	if team == 1:
		return b_team2_owns_prerequisites
	return false

# AControlPoint::SetCapturingTeam rva=0x1515b30 / SetOwningTeam rva=0x1515c30: on change, the Blueprint event
func set_capturing_team(t: int) -> void:
	if capturing_team != t:
		capturing_team = t
		on_capturing_team_changed()

func set_owning_team(t: int) -> void:
	if owning_team != t:
		owning_team = t
		on_owning_team_changed()

# OnCapturingTeamChanged / OnOwningTeamChanged (BlueprintNativeEvent; native _Implementation rva=0x1508070 updates the
# visuals and UI widgets on clients). The adapter shows announcements from these events (local_announcement).
func on_capturing_team_changed() -> void:
	_ev("capturing_team", {"team": capturing_team})

func on_owning_team_changed() -> void:
	_ev("owning_team", {"team": owning_team})

# ---- presence --------------------------------------------------------------------------------------------------
# OnCaptureAreaBeginOverlap rva=0x1507fa0: a character -> UpdatePresenceNumbers; its CurrentCapturePoint = this and
# CurrentCapturePointTime = 0
func begin_overlap(c) -> void:
	if not overlapping.has(c):
		overlapping.append(c)
	update_presence_numbers()
	c.capture_point = self
	c.capture_point_time = 0.0

# OnCaptureAreaEndOverlap rva=0x1508000: UpdatePresenceNumbers; if its CurrentCapturePoint is this, both cleared
func end_overlap(c) -> void:
	overlapping.erase(c)
	update_presence_numbers()
	if c.capture_point == self:
		c.capture_point = null
		c.capture_point_time = 0.0

# AControlPoint::UpdatePresenceNumbers rva=0x151e920: OverlapsCache = the overlapping MordhauCharacters; each one not
# bIsDead (+0x504) counts for Team1Presence (character Team +0x660 == 0) or Team2Presence (== 1)
func update_presence_numbers() -> void:
	team1_presence = 0
	team2_presence = 0
	overlaps_cache = overlapping.duplicate()
	for c in overlaps_cache:
		if c == null or not c.alive:
			continue
		if c.team == 0:
			team1_presence += 1
		elif c.team == 1:
			team2_presence += 1

# ---- tick ------------------------------------------------------------------------------------------------------
# AControlPoint::Tick rva=0x1517270. match_in_progress: the authority game mode's AGameMode::IsMatchInProgress
# (vtable +0x820 call at 0x14151740f)
func tick(dt: float, now: float, match_in_progress: bool) -> void:
	_bp_tick(now)
	# prerequisites: every Team1PrerequisitePoint owned by team 0 (every Team2... by team 1); empty = true
	var t1 := true
	for p in team1_prerequisites:
		if p != null and p.owning_team != 0:
			t1 = false
			break
	var t2 := true
	for p in team2_prerequisites:
		if p != null and p.owning_team != 1:
			t2 = false
			break
	if t1 != b_team1_owns_prerequisites:
		b_team1_owns_prerequisites = t1
		if owning_team == 1:
			_ev("enemy_gained_prerequisites" if t1 else "enemy_lost_prerequisites")
	if t2 != b_team2_owns_prerequisites:
		b_team2_owns_prerequisites = t2
		if owning_team == 0:
			_ev("enemy_gained_prerequisites" if t2 else "enemy_lost_prerequisites")
	var old := capture_progress
	if authority:
		if match_in_progress:
			update_capture_progress(dt)
	else:
		var target := float(replicated_capture_progress) * REP_INV
		capture_progress = (dt / d.network_smooth_time) * (target - old) + old
		if absf(capture_progress - target) <= SNAP_EPS:
			capture_progress = target
	if absf(capture_progress - old) <= UNCHANGED_EPS:
		unchanged_capture_progress_time += dt
		if authority and unchanged_capture_progress_time > NET_IDLE_AFTER_S:
			net_update_frequency = NET_UPDATE_FREQUENCY_IDLE
	else:
		unchanged_capture_progress_time = 0.0
		if authority:
			net_update_frequency = NET_UPDATE_FREQUENCY_ACTIVE
		_ev("update_visuals", {"progress": capture_progress})
	if not dedicated:
		var flash := unchanged_capture_progress_time >= FLASH_AFTER_S
		if b_is_flashing == flash:
			b_is_flashing = not flash
			_ev("started_flashing" if b_is_flashing else "stopped_flashing")

# AControlPoint::UpdateCaptureProgress rva=0x151b3f0
func update_capture_progress(dt: float) -> void:
	if not b_is_capturable:
		return
	var p1 := team1_presence
	var p2 := team2_presence
	var t1 := p1 >= 1 and b_team1_owns_prerequisites
	var t2 := p2 >= 1 and b_team2_owns_prerequisites
	if t1 and t2:
		if p1 == p2:
			return
		if d.b_should_pause_capture_if_enemy_near:
			return
	var team1_wins := t1 and p1 > p2
	var team2_wins := t2 and p1 < p2
	var nobody := not (team1_wins or team2_wins)
	var lead := 0.0
	if team1_wins:
		lead = float(p1 - p2)
	elif team2_wins:
		lead = float(p2 - p1)
	var cap_speed := _curve(d.capture_speed_curve, lead)
	var neu_speed := _curve(d.neutralize_speed_curve, lead)
	if nobody and owning_team == NO_TEAM:
		set_capture_progress(capture_progress - dt * d.uncapture_speed, NO_TEAM, false, dt)
		return
	# the owner is challenged: Owning 0 -> team 2 leads; Owning 1 -> team 1 leads; unowned -> always
	var challenged := team2_wins if owning_team == 0 else (team1_wins if owning_team == 1 else true)
	if not challenged:
		set_capture_progress(cap_speed * dt + capture_progress, owning_team, true, dt)
		return
	var cap := capturing_team
	if cap != 0 and team1_wins:
		set_capture_progress(capture_progress - neu_speed * dt, 0, true, dt)
	elif cap != 0 and cap != 1 and team2_wins:
		set_capture_progress(capture_progress - neu_speed * dt, 1, true, dt)
	elif cap == 0 and team2_wins:
		set_capture_progress(capture_progress - neu_speed * dt, 1, true, dt)
	else:
		set_capture_progress(cap_speed * dt + capture_progress, cap, true, dt)

# UCurveFloat::GetFloatValue on the point's curve (none -> 0)
static func _curve(path: String, t: float) -> float:
	if path == "":
		return 0.0
	return CombatData.curve_value(path, t)

# AControlPoint::SetCaptureProgress rva=0x1515740 (authority only). dt: World DeltaTimeSeconds (score intervals)
func set_capture_progress(p: float, new_captor: int, award: bool, dt: float) -> void:
	if not authority:
		return
	var old := capture_progress
	p = clampf(p, 0.0, 1.0)
	var score := d.award_score_capturing
	if p < old:
		score = d.award_score_neutralizing
	capture_progress = p
	var reason := REASON_NEUTRALIZING
	if old <= p:
		reason = REASON_CAPTURING
	if p != 0.0:
		if p == 1.0:
			set_capturing_team(new_captor)
			if old == capture_progress and owning_team == capturing_team:
				return
			score = d.award_score_captured
			reason = REASON_CAPTURED
			capture_progress = 1.0
			set_owning_team(capturing_team)
		elif old == p:
			return
	else:
		if old == p and owning_team == NO_TEAM:
			if capturing_team == new_captor:
				return
			score = d.award_score_neutralized
			award = false
		else:
			score = d.award_score_neutralized
		set_capturing_team(new_captor)
		reason = REASON_NEUTRALIZED
		set_owning_team(NO_TEAM)
	unchanged_capture_progress_time = 0.0
	if award:
		_award(new_captor, score, reason, dt)
	update_spawns()
	replicated_capture_progress = int(capture_progress * REP_SCALE) & 0xff

# SetCaptureProgress's score loop over OverlapsCache: a live character with a MordhauPlayerState on the captor's team
# accumulates CurrentCapturePointTime += DeltaTime; captured / neutralized (reasons 9, 11) award at once and reset the
# time, capturing / neutralizing award each time floor(time / AwardScoreInterval) steps up; AddScore +
# ClientReceiveScoreNoState(reason, score) when score != 0. A character of another team has its time reset.
func _award(captor: int, score: int, reason: int, dt: float) -> void:
	for c in overlaps_cache:
		if c == null or not c.alive:
			continue
		if c.team != captor:
			c.capture_point_time = 0.0
			continue
		var iv := d.award_score_interval
		if absf(iv) <= INTERVAL_EPS:
			continue
		var before: float = c.capture_point_time
		c.capture_point_time = dt + before
		var give := false
		if reason == REASON_CAPTURED or reason == REASON_NEUTRALIZED:
			c.capture_point_time = 0.0
			give = score != 0
		else:
			give = int(before / iv) < int(c.capture_point_time / iv) and score != 0
		if give:
			if add_score.is_valid():
				add_score.call(c, score)
			_ev("score", {"who": c.name, "reason": reason, "score": score})

# AControlPoint::UpdateSpawns rva=0x151fb90: spawns enabled for OwningTeam when owned and (fully captured or not
# bPreventSpawningIfContested); on a change, each SpawnPoint: bIsSpawnDisabled = bSpawnsDisabled, or true when the
# start's Team differs from SpawnsTeam
func update_spawns() -> void:
	var disable := true
	if (capture_progress == 1.0 or not d.b_prevent_spawning_if_contested) and owning_team != NO_TEAM:
		disable = false
	var team := owning_team
	var team_changed := owning_team != NO_TEAM and spawns_team != team
	if disable != b_spawns_disabled or team_changed:
		b_spawns_disabled = disable
		spawns_team = team
		for s in spawn_points:
			if s == null:
				continue
			s.b_is_spawn_disabled = b_spawns_disabled
			if s.team != spawns_team:
				s.b_is_spawn_disabled = true

# AControlPoint::OnRep_ReplicatedCaptureProgress rva=0x150ecf0 (client): the first replication snaps the progress
func on_rep_replicated_capture_progress() -> void:
	if not b_has_ever_replicated_progress:
		unchanged_capture_progress_time = UNCHANGED_AT_CTOR
		b_has_ever_replicated_progress = true
		capture_progress = float(replicated_capture_progress) * REP_INV

# ---- BP_SkirmishCapturePoint -----------------------------------------------------------------------------------
func is_skirmish_point() -> bool:
	return d.time_to_unlock >= 0.0

# RoundStarted: SetCaptureProgress(0, 255, false); Locked = true (OnRep_Locked: bIsCapturable = !Locked);
# RetriggerableDelay(TimeToUnlock) then Locked = false (header)
func round_started(now: float) -> void:
	var r: Array = _ks.call("skm_cp_round_reset")
	set_capture_progress(float(r[0]), int(r[1]), bool(r[2]), 0.0)
	_set_locked(true)
	_unlock_at = now + d.time_to_unlock

func _set_locked(v: bool) -> void:
	locked = v
	b_is_capturable = not locked
	_ev("locked", {"locked": locked})

func _bp_tick(now: float) -> void:
	if _unlock_at >= 0.0 and now >= _unlock_at:
		_unlock_at = -1.0
		_set_locked(false)

# BP_SkirmishCapturePoint OnCapturingTeamChanged on the local machine (header): the announcement for local team
# `local_team` at time `now`, or {} (relevance 2 = spectator / no team, or too early, or no captor)
func local_announcement(e: Dictionary, local_team: int, now: float) -> Dictionary:
	if e.kind != "capturing_team" or not is_skirmish_point():
		return {}
	if not (now > float(_ks.call("skm_cp_announce_after_s"))) or int(e.team) == NO_TEAM:
		return {}
	var rel := int(_ks.call("skm_cp_no_relevance"))
	if local_team == 0 or local_team == 1:
		rel = 1 if int(e.team) != local_team else 0
	if rel == 0:
		return {"kind": "announce", "text": String(_ks.call("skm_cp_we_capturing")), "subtext": "",
			"duration": float(_ks.call("skm_cp_announce_s"))}
	if rel == 1:
		return {"kind": "announce", "text": String(_ks.call("skm_cp_enemy_capturing")), "subtext": "",
			"duration": float(_ks.call("skm_cp_announce_s"))}
	return {}

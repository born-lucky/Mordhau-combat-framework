# mordhau_player_view.gd - the local player's side of the game state (BP_MordhauGameState RepNotify / multicast
# handlers), engine-agnostic: turns the mode's events into HUD commands, as the Blueprints call the HUD:
#   ReceiveKillNotify (the ReplicatedKillNotify multicast): KilledWith = the weapon's EquipmentName (Flags 0), "Kick"
#     (1), "Fall" (2); no equipment -> vehicle / projectile name, else "RIP"; HUD.SendMessageToKillFeed(Killer,
#     KilledWith, Killed)                                              -> {kind: "kill_feed", killer, with, victim}
#     the local player killed -> ShowKilledBy (not drawn)              -> {kind: "killed_by", killer}
#   OnRep_MatchEndInfo -> HandleMatchEndInfo, which each mode's game state overrides (MordhauGameMode.match_result_for:
#     FfaMode, TdmMode, SkmMode)                                       -> {kind: "match_result", victory, text, subtext}
# Texts: ModeKismet (kf_*). Colours: GetKillfeedColor (kill_feed_color()). Mode-specific client logic (round
# announcements) comes from the mode as extra commands (SkmMode.client_tick).
class_name MordhauPlayerView
extends RefCounted

var d: MordhauModeData
var me: MordhauGameMode.Ctrl
var mode: MordhauGameMode
var out: Array = []
var _end_seen := {}

func _init(m: MordhauGameMode, local: MordhauGameMode.Ctrl) -> void:
	mode = m
	me = local
	d = m.d

func drain() -> Array:
	var o := out
	out = []
	return o

# feed the mode's events of this tick (before the adapter drains them for itself)
func on_events(evs: Array) -> void:
	for e in evs:
		match String(e.kind):
			"kill_notify": _kill_notify(e)
			"match_end_info": _end_info(e)
			"announce": out.append(e)		# the game state's own RepNotify announcements (SkmMode)

# the game state's client tick (BP ReceiveTick on the local machine; SkmMode)
func tick() -> void:
	out.append_array(mode.client_tick(me))

func _end_info(e: Dictionary) -> void:
	if e == _end_seen:
		return
	_end_seen = e
	var r := mode.match_result_for(me, e)
	if not r.is_empty():
		out.append(r)

func _kill_notify(e: Dictionary) -> void:
	var with_text := ""
	match int(e.flags):
		0: with_text = String(e.weapon) if String(e.weapon) != "" else String(d.kv("kf_rip_text"))
		1: with_text = String(d.kv("kf_kick_text"))
		2: with_text = String(d.kv("kf_fall_text"))
	var killer := mode.by_name(String(e.killer))
	var victim := mode.by_name(String(e.killed))
	out.append({"kind": "kill_feed", "killer": String(e.killer), "with": with_text, "victim": String(e.killed),
		"killer_color": kill_feed_color(killer), "victim_color": kill_feed_color(victim)})
	if victim == me and killer != me:
		out.append({"kind": "killed_by", "killer": String(e.killer)})

# BP_MordhauGameState GetKillfeedColor(PlayerState): the local player -> gold; team mode: team 0 red, team 1 blue;
# else neutral (linear colours from the bytecode)
func kill_feed_color(c: MordhauGameMode.Ctrl) -> Color:
	if c == me:
		return ModeKismet.color("kf_local_color")
	if d.state.b_is_team_mode and c != null:
		if c.team == 0: return ModeKismet.color("kf_team0_color")
		if c.team == 1: return ModeKismet.color("kf_team1_color")
	return ModeKismet.color("kf_neutral_color")

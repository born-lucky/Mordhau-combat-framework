# duel_player_view.gd - the local player's side of BP_DuelPlayerController (client logic only), engine-agnostic:
# reads the controller's ReplicatedRoomGame (MordhauGameMode.Ctrl) and turns it into HUD commands, as the Blueprint calls
# HUD.ShowAnnouncement / HUD.ShowMatchResult / PlaySound2D:
#   {kind: "announce", text, subtext, duration}   -> BP_MordhauHUD.ShowAnnouncement -> BP_Announcement.Show
#   {kind: "match_result", victory, text, subtext} -> BP_MordhauHUD.ShowMatchResult -> BP_Victory/DefeatPopup
#   {kind: "sound", name}  {kind: "hide_main_menu"}  {kind: "hide_emote_menu"}  {kind: "allow_drop"}
#   {kind: "end_screen"} (ShowDuelEndscreenDelayed, after OnRep_NewMMR)
# Every literal (texts, durations, thresholds) is a bytecode constant from DuelMode.DuelData (scripts/mode_kismet.py).
#
# OnRep_ReplicatedRoomGame: the server calls it explicitly every TickRoom after copying the game (BP_DuelGameMode
# TickRoom 573), but on a dedicated server no controller is local, so only clients act on it, and a client's RepNotify
# runs when the replicated value changes. This view is that client: on_rep() runs when the copied game differs from
# the last one seen (UNCONFIRMED: replication semantics, not the shipped listen-server path).
class_name DuelPlayerView
extends RefCounted

var d: DuelMode.DuelData
var c: MordhauGameMode.Ctrl
var last_game := {}
var countdown := 0					# BP_DuelPlayerController.Countdown
var loadout_selection_timer := 0.0	# BP_ProfileCustomization.LoadoutSelectionTimer (no CDO default -> 0)
var out: Array = []
var end_screen_at := -1.0

func _init(ctrl: MordhauGameMode.Ctrl, data: DuelMode.DuelData = null) -> void:
	c = ctrl
	d = data if data != null else DuelMode.DuelData.shared()

func drain() -> Array:
	var o := out
	out = []
	return o

func _announce(key: String, subtext := "") -> void:
	var v: Array = d.kv(key)
	out.append({"kind": "announce", "text": String(v[0]), "subtext": subtext, "duration": float(v[1]), "src": d.src(key)})

func _count_text(n: int) -> String:
	return String(d.kv("pc_count_format")).replace("{0}", str(n))		# Format("-{0}-", ArgumentValueInt)

# ReceiveTick (ubergraph 713). `now` = GetTimeSeconds on the client, `server_now` = GameState GetServerWorldTimeSeconds
# (one process: the same clock).
func tick(now: float, server_now: float) -> void:
	if c.has_replicated and c.replicated_room_game != last_game:
		on_rep()
		last_game = c.replicated_room_game.duplicate(true)
	if end_screen_at >= 0.0 and now >= end_screen_at:
		end_screen_at = -1.0
		out.append({"kind": "end_screen"})
	if not (c.has_replicated and now > float(d.kv("pc_min_time_s"))):
		return
	var rnd: Dictionary = c.replicated_room_game.round
	var st: int = rnd.stage
	if st == d.stage.WaitingForPlayers:
		# ShowAnnouncement("Waiting for players", Format("-{0}-", FCeil(FMax(StartTime - ServerTime, 0))), 0.1)
		var v: Array = d.kv("pc_waiting_text")
		out.append({"kind": "announce", "text": String(v[0]), "duration": float(v[1]), "src": d.src("pc_waiting_text"),
			"subtext": _count_text(int(ceil(maxf(float(rnd.start_time) - server_now, 0.0))))})
	elif st == d.stage.WaitingToStart:
		# LoadoutSelectionTimer = FMax(timer, StartTime - ServerTime + now + 5): the 5 s RoundStart that follows
		loadout_selection_timer = maxf(loadout_selection_timer,
			float(rnd.start_time) - server_now + now + float(d.kv("pc_loadout_extra_s")))
		update_countdown(loadout_selection_timer, now)
	elif st == d.stage.RoundStart:
		loadout_selection_timer = maxf(float(rnd.start_time) - server_now + now, loadout_selection_timer)
		update_countdown(loadout_selection_timer, now)
	elif st == d.stage.RoundPlay:
		countdown = 0

# UpdateCountdown(StartTime): Countdown = Max(FCeil(StartTime - now), 0); on change: <= 10 PlaySound2D(UI_ClockTickCue);
# <= 5 "Get ready!" else "Match starting", subtext "-N-", 1.2 s
func update_countdown(start_time: float, now: float) -> void:
	var n := maxi(int(ceil(start_time - now)), 0)
	if n == countdown:
		return
	countdown = n
	if countdown <= int(d.kv("pc_tick_sound_at")):
		out.append({"kind": "sound", "name": "UI_ClockTickCue"})
	if countdown <= int(d.kv("pc_get_ready_at")):
		_announce("pc_get_ready", _count_text(countdown))
	else:
		_announce("pc_match_starting", _count_text(countdown))

# OnRep_ReplicatedRoomGame: HasReplicatedRoomGame = true; switch Stage:
#   RoundStart: hide the emote menu.  RoundPlay: "Fight!" 2 s, HideMainMenu, PlaySound2D(EnemyCapped).
#   RoundEnd: Winner == my team -> (a team has 5 wins ? ShowMatchResult(true, "victory") : "Round won" 3 s)
#             else (5 wins ? ShowMatchResult(false, "defeat") : "Round lost" 3 s)
#   then: Stage == RoundPlay -> pawn bAllowDrop = true
func on_rep() -> void:
	var g: Dictionary = c.replicated_room_game
	var st: int = g.round.stage
	if st == d.stage.RoundStart:
		out.append({"kind": "hide_emote_menu"})
	elif st == d.stage.RoundPlay:
		_announce("pc_fight")
		out.append({"kind": "hide_main_menu"})
		out.append({"kind": "sound", "name": "EnemyCapped"})
	elif st == d.stage.RoundEnd:
		var over: bool = int(g.team1_wins) == int(d.kv("pc_match_over_wins")) or int(g.team2_wins) == int(d.kv("pc_match_over_wins"))
		var won: bool = int(g.round.winner) == (c.team & 0xff)
		if over:
			out.append({"kind": "match_result", "victory": won, "text": String(d.kv("pc_victory" if won else "pc_defeat")),
				"subtext": "", "src": d.src("pc_victory" if won else "pc_defeat")})
		else:
			_announce("pc_round_won" if won else "pc_round_lost")
	if st == int(d.kv("pc_drop_stage")):
		out.append({"kind": "allow_drop"})

# OnRep_NewMMR -> OnReceivedMMR + ShowDuelEndscreenDelayed (Delay 6.5 s -> HUD.ShowDuelEndScreen)
func on_new_mmr(now: float) -> void:
	end_screen_at = now + float(d.kv("pc_endscreen_delay_s"))

# BP_DuelGameState GetScoreboardTeamObjectiveValue(Team): the local controller's ReplicatedRoomGame wins as text
func team_wins(team: int) -> int:
	if not c.has_replicated:
		return 0
	return int(c.replicated_room_game.team1_wins if team == 0 else c.replicated_room_game.team2_wins)

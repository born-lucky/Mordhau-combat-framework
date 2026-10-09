# user_settings.gd - the game's settings screen model: UMordhauGameUserSettings (MordhauGameUserSettings.h), engine-
# agnostic. The fields, their defaults, the slider ranges, the accessors and the HUD visibility queries are ported from
# extract/native/decomp/UMordhauGameUserSettings.cpp; nothing is typed in that the exe does not hold.
#
#   set_to_defaults()   UMordhauGameUserSettings::SetToDefaults rva=0x15a7c90 (no DefaultGameUserSettings.ini ships in
#                       extract/config, so these are the values a fresh install starts with)
#   limits(name)        the Get*Limits functions (FVector2D min / max of each settings-screen slider)
#   get_setting(name) / set_setting(name, value)
#                       the Get* / Set* UFUNCTIONs: plain loads / stores, no clamping. The exe's quirk is kept:
#                       GetCombatHeadbob and GetMovementHeadbob are one function (ICF, 0x15952f0) that reads +0x190
#                       MovementHeadbob, while SetCombatHeadbob writes +0x194 CombatHeadbob.
#   apply()             UMordhauGameUserSettings::ApplyNonResolutionSettings rva=0x1582870: copies the gameplay / HUD
#                       fields into their console variables (HideHUD is NOT among them: only the console sets it)
#   should_show(name)   the ShouldShow* queries read the console variables, not the fields: HUD element = HideHUD == 0
#                       and Show<X> != 0; ShouldShowHUD / ShouldDrawTracers / ShouldShowBlood / ... one cvar each
# Console variable defaults are the RegisterConsoleVariable values in the static initializers (_static_init.cpp).
# Not ported: the engine part of UGameUserSettings (resolution, scalability), SaveSettings / LoadSettings (ini file),
# the language pick from the OS culture (SetToDefaults sets "English", then the OS culture's display name if it is one
# of AvailableLanguages - here the game's culture list is not read: UNCONFIRMED, "English" kept), favourite / recent
# server lists and per-mode player counts, matchmaking.
class_name UserSettings
extends RefCounted

var v := {}			# field (snake_case of the UPROPERTY) -> value
var cvars := {}		# console variable (snake_case, without the CVar prefix) -> value
var version := 0				# UGameUserSettings::Version
var mordhau_version := 0		# MordhauVersion [+0x158]

func _init() -> void:
	_cvar_defaults()
	set_to_defaults()

# RegisterConsoleVariable(name, default, help, flags) in each CVar's dynamic initializer (_static_init.cpp).
func _cvar_defaults() -> void:
	cvars = {
		"hide_hud": 0,					# CVarHideHUD initializer rva=0x640390
		"show_ammo": 1,					# CVarShowAmmo initializer rva=0x640db0
		"show_announcements": 1,		# CVarShowAnnouncements initializer rva=0x640e30
		"show_chat_box": 1,				# CVarShowChatBox initializer rva=0x640eb0
		"show_combat_hints": 1,			# CVarShowCombatHints initializer rva=0x640f30
		"show_emotes_menu": 1,			# CVarShowEmotesMenu initializer rva=0x640fb0
		"show_equipment": 1,			# CVarShowEquipment initializer rva=0x641030
		"show_hit_marker": 1,			# CVarShowHitMarker initializer rva=0x6410b0
		"show_kill_feed": 1,			# CVarShowKillFeed initializer rva=0x641130
		"show_killed_by": 1,			# CVarShowKilledBy initializer rva=0x6411b0
		"show_objectives": 1,			# CVarShowObjectives initializer rva=0x6412a0
		"show_score_feed": 1,			# CVarShowScoreFeed initializer rva=0x641400
		"show_server_in_scoreboard": 1,	# CVarShowServerInScoreboard initializer rva=0x641480
		"show_spawn_info": 1,			# CVarShowSpawnInfo initializer rva=0x641570
		"show_status_bar": 1,			# CVarShowStatusBar initializer rva=0x6415f0
		"show_target_info": 1,			# CVarShowTargetInfo initializer rva=0x641670
		"show_tips": 1,					# CVarShowTips initializer rva=0x6416f0
		"crosshair_type": 0,			# CVarCrosshairType initializer rva=0x63fe00
		"draw_tracers": 0,				# CVarDrawTracers initializer rva=0x63ff50
		"show_matchmaking_debug": 0,	# CVarShowMatchmakingDebug initializer rva=0x641230
		"show_observed_delay": 0,		# CVarShowObservedDelay initializer rva=0x641320
		"gore": 2,						# CVarGore initializer rva=0x640290
	}

# UMordhauGameUserSettings::SetToDefaults rva=0x15a7c90 (after UGameUserSettings::SetToDefaults). Field order as stored.
func set_to_defaults() -> void:
	v["language"] = "English"			# FString "English" (then the OS culture, see header)
	v["gore"] = 2
	v["profanity_filter"] = 1
	v["third_person_death_camera"] = 0
	v["character_cloth"] = 2
	v["friendly_markers"] = 0
	v["no_team_colors_on_gear"] = 0
	v["headbob"] = 1.0
	v["movement_headbob"] = 1.0
	v["combat_headbob"] = 1.0
	v["max_ragdolls"] = 10
	v["ragdoll_stay_time"] = 30.0
	v["mouse_smoothing"] = 0.0
	v["draw_tracers"] = 0
	v["draw_tracers_stay_time"] = 2.0
	v["force_feedback"] = false
	v["crossplay_enabled"] = true
	v["ranged_sensitivity"] = 0.35
	v["hide_hud"] = 0
	v["hide_default_loadouts"] = 0
	v["show_server_in_scoreboard"] = 1
	v["crosshair_type"] = 0
	v["show_killed_by"] = 1
	v["show_status_bar"] = 1
	v["show_target_info"] = 1
	v["show_spawn_info"] = 1
	v["show_chat_box"] = 1
	_defaults_2()
	_defaults_3()

# SetToDefaults rva=0x15a7c90, continued
func _defaults_2() -> void:
	v["show_emotes_menu"] = 1
	v["show_equipment"] = 1
	v["show_ammo"] = 1
	v["show_announcements"] = 1
	v["show_tips"] = 1
	v["show_objectives"] = 1
	v["show_hit_marker"] = 1
	v["show_score_feed"] = 1
	v["show_combat_hints"] = 1
	v["show_kill_feed"] = 1
	v["show_observed_delay"] = 0
	v["screen_percentage"] = 100.0
	v["fullscreen_mode"] = 0				# UGameUserSettings FullscreenMode
	v["frame_rate_limit"] = 60.0			# UGameUserSettings FrameRateLimit
	v["use_vsync"] = false					# UGameUserSettings bUseVSync
	v["field_of_view"] = 78.0
	v["camera_distance"] = 0.0
	v["gamma"] = 1.0
	v["anti_aliasing"] = 2
	v["indirect_capsule_shadows"] = 1
	v["character_fidelity"] = 2
	v["ragdoll_fidelity"] = 1
	v["bloom"] = 1.0
	v["motion_blur"] = 0.09
	v["screen_space_reflections"] = 1
	v["ambient_occlusion"] = 1
	v["lens_flares"] = 1

# The game-mode names are FName strings in .rdata (NAME_InvasionFrontline "Invasion & Frontline", NAME_Teamfight
# "Teamfight", NAME_GameModeFilterAll "All"; tests/test_ui_settings.gd checks the bytes).
func _defaults_3() -> void:
	v["platform_specific"] = true		# the return of `IAccessibleProperty::IsReadOnly` 0x7bf3e0 (ICF "mov al, 1; ret")
	# SetToDefaults rva=0x15a7c90, continued
	v["master_volume"] = 1.0
	v["effects_volume"] = 1.0
	v["music_volume"] = 0.5
	v["video_volume"] = 1.0
	v["voice_volume"] = 1.0
	v["instruments_volume"] = 1.0
	v["casual_matchmaking_region"] = 9
	v["casual_matchmaking_game_modes"] = PackedStringArray(["Invasion & Frontline"])
	v["ranked_matchmaking_region"] = 9
	v["ranked_matchmaking_game_modes"] = PackedStringArray(["Teamfight"])
	v["server_browser_is_official"] = false
	v["server_browser_console_server"] = false
	v["server_browser_not_full"] = true
	v["server_browser_has_players"] = false
	v["server_browser_no_password"] = true
	v["server_browser_game_mode"] = "All"
	v["server_browser_max_ping"] = 100
	# not written by SetToDefaults: zero-initialised UObject memory, the constructor leaves them (UNCONFIRMED that no
	# config overrides them)
	for k in ["nvidia_reflex", "server_type_filter", "server_population_filter", "server_modded_filter",
			"server_password_filter"]:
		if not v.has(k):
			v[k] = 0
	if not v.has("is_psn_lock_enabled"):
		v["is_psn_lock_enabled"] = false	# ctor rva=0x157f020: bIsPSNLockEnabled = false

# The settings-screen slider ranges, FVector2D(min, max) per Get*Limits (each "mov dword [rdx], imm; mov dword
# [rdx+4], imm"). GetGammaLimits, GetHeadbobLimits, GetMovementHeadbobLimits and GetCombatHeadbobLimits are one
# function (ICF 0x1595300): 0..2.
func limits(name: String) -> Vector2:
	match name:
		"bloom": return Vector2(0.0, 1.0)						# `GetBloomLimits` 0x9d8030
		"camera_distance": return Vector2(-15.0, 15.0)			# `GetCameraDistanceLimits` 0x15951c0
		"combat_headbob", "headbob", "movement_headbob", "gamma":
			return Vector2(0.0, 2.0)							# `GetCombatHeadbobLimits` 0x1595300
		"field_of_view": return Vector2(30.0, 101.0)			# `GetFieldOfViewLimits` 0x1595df0
		"frame_rate": return Vector2(30.0, 250.0)				# `GetFrameRateLimits` 0x1595e20
		"motion_blur": return Vector2(0.0, 0.3)					# `GetMotionBlurLimits` 0x1596cc0
		"mouse_smoothing": return Vector2(0.0, 4.0)				# `GetMouseSmoothingLimits` 0x1596d10
		"ranged_sensitivity": return Vector2(0.1, 1.0)			# `GetRangedSensitivityLimits` 0x1598b10
		"screen_percentage": return Vector2(25.0, 200.0)		# `GetScreenPercentageLimits` 0x1598d40
		"tracers_stay_time": return Vector2(1.0, 10.0)			# `GetTracersStayTimeLimits` 0x1599d10
	push_error("UserSettings: no limits for " + name)
	return Vector2.ZERO

func max_ragdolls_limit() -> int:
	return 20				# `GetMaxRagdollsLimit` 0x1596ca0: mov eax, 0x14

func ragdoll_stay_time_limit() -> float:
	return 120.0			# `GetRagdollStayTimeLimit` 0x1598af0: movss from .rdata (120.0)

# `SetDefaultRangedSensitivity` 0x15a71e0: RangedSensitivity = 0.35
func set_default_ranged_sensitivity() -> void:
	v["ranged_sensitivity"] = 0.35

func get_setting(name: String) -> Variant:
	return v[GETTERS[name]]

func set_setting(name: String, value) -> void:
	v[SETTERS[name]] = value

# `GetServerFilterValue` 0x1599500 / `SetServerFilter` 0x15a7790: category 0 ServerTypeFilter, 1 ServerPopulationFilter,
# 2 ServerModdedFilter, 3 ServerPasswordFilter; any other category reads 0 and writes nothing.
const SERVER_FILTERS := ["server_type_filter", "server_population_filter", "server_modded_filter", "server_password_filter"]

func get_server_filter_value(category: int) -> int:
	return int(v[SERVER_FILTERS[category]]) if category >= 0 and category < SERVER_FILTERS.size() else 0

func set_server_filter(category: int, value: int) -> void:
	if category >= 0 and category < SERVER_FILTERS.size():
		v[SERVER_FILTERS[category]] = value

# UMordhauGameUserSettings::UpdateVersion rva=0x15ad990: MordhauVersion = 1, Version = 5.
func update_version() -> void:
	mordhau_version = 1
	version = 5

# UMordhauGameUserSettings::IsVersionValid rva=0x159c210: UGameUserSettings::IsVersionValid() and (MordhauVersion == 1
# or MordhauVersion != 0). The engine check is Version == UE_GAMEUSERSETTINGS_VERSION; its value 5 is what
# UpdateVersion stores (UGameUserSettings::IsVersionValid itself not disassembled: UNCONFIRMED).
func is_version_valid() -> bool:
	if version != 5:
		return false
	return mordhau_version != 0

# ApplyNonResolutionSettings rva=0x1582870: each of these fields is printed ("%d" / "%g") into its console variable
# with ECVF_SetByGameSetting (0x2000000), in this order. HideHUD is not applied.
const APPLIED := ["gore", "profanity_filter", "third_person_death_camera", "character_cloth", "friendly_markers",
	"no_team_colors_on_gear", "headbob", "movement_headbob", "combat_headbob", "max_ragdolls", "ragdoll_stay_time",
	"mouse_smoothing", "draw_tracers", "draw_tracers_stay_time", "show_server_in_scoreboard", "crosshair_type",
	"show_killed_by", "show_status_bar", "show_target_info", "show_spawn_info", "show_chat_box", "show_emotes_menu",
	"show_equipment", "show_ammo", "show_announcements", "show_tips", "show_objectives", "show_hit_marker",
	"show_score_feed", "show_kill_feed", "show_combat_hints", "show_observed_delay", "screen_percentage",
	"field_of_view", "camera_distance", "gamma", "character_fidelity", "ragdoll_fidelity", "bloom", "motion_blur"]

func apply() -> void:
	for k in APPLIED:
		cvars[k] = v[k]

# ShouldShow<X> (76 bytes each): CVarHideHUD == 0 and CVarShow<X> != 0.
const HUD_ELEMENTS := {
	"ammo": "show_ammo",					# `ShouldShowAmmo` 0x15a8470
	"announcements": "show_announcements",	# `ShouldShowAnnouncements` 0x15a84c0
	"chat_box": "show_chat_box",			# `ShouldShowChatBox` 0x15a8540
	"emotes_menu": "show_emotes_menu",		# `ShouldShowEmotesMenu` 0x15a8590
	"equipment": "show_equipment",			# `ShouldShowEquipment` 0x15a85e0
	"hit_marker": "show_hit_marker",		# `ShouldShowHitMarker` 0x15a8660
	"kill_feed": "show_kill_feed",			# `ShouldShowKillFeed` 0x15a86b0
	"killed_by": "show_killed_by",			# `ShouldShowKilledBy` 0x15a8700
	"objectives": "show_objectives",		# `ShouldShowObjectives` 0x15a8780
	"score_feed": "show_score_feed",		# `ShouldShowScoreFeed` 0x15a8800
	"spawn_info": "show_spawn_info",		# `ShouldShowSpawnInfo` 0x15a8880
	"status_bar": "show_status_bar",		# `ShouldShowStatusBar` 0x15a88d0
	"target_info": "show_target_info",		# `ShouldShowTargetInfo` 0x15a8920
	"tips": "show_tips",					# `ShouldShowTips` 0x15a8970
}

func should_show(element: String) -> bool:
	return int(cvars["hide_hud"]) == 0 and int(cvars[HUD_ELEMENTS[element]]) != 0

# One console variable each (39 bytes): `ShouldShowHUD` 0x15a8630 HideHUD == 0; `ShouldDrawTracers` 0x15a8390
# DrawTracers != 0; `ShouldShowBlood` 0x15a8510 Gore != 0; `ShouldShowObservedDelay` 0x15a87d0;
# `ShouldShowServerInScoreboard` 0x15a8850; `ShouldShowMatchmakingDebug` 0x15a8750 (each != 0).
func should_show_hud() -> bool:
	return int(cvars["hide_hud"]) == 0

func should_draw_tracers() -> bool:
	return int(cvars["draw_tracers"]) != 0

func should_show_blood() -> bool:
	return int(cvars["gore"]) != 0

func should_show_observed_delay() -> bool:
	return int(cvars["show_observed_delay"]) != 0

func should_show_server_in_scoreboard() -> bool:
	return int(cvars["show_server_in_scoreboard"]) != 0

func should_show_matchmaking_debug() -> bool:
	return int(cvars["show_matchmaking_debug"]) != 0

# `GetActualCrosshairType` 0x1594aa0: CVarCrosshairType's value (GetCrosshairType returns the field).
func get_actual_crosshair_type() -> int:
	return int(cvars["crosshair_type"])

# `ShouldQuickSpawn` 0x7bf520 / `ShouldShowWatermark` 0x7bf520: "xor al, al; ret" - always false.
# `GetQuickSpawn` 0x7bf3a0: "xor eax, eax" - 0. `GetHideWatermark` 0x809a90: "mov eax, 1" - 1.
# `SetQuickSpawn` 0x7bf350 / `SetHideWatermark` 0x7bf350: "ret 0" - store nothing.
func should_quick_spawn() -> bool:
	return false

func should_show_watermark() -> bool:
	return false

func get_quick_spawn() -> int:
	return 0

func get_hide_watermark() -> int:
	return 1

# Get<X> -> the field it loads, Set<X> -> the field it stores (each a single mov; snake_case names).
const GETTERS := {
	"ambient_occlusion": "ambient_occlusion",	# `GetAmbientOcclusion` 0x1594ca0
	"anti_aliasing": "anti_aliasing",	# `GetAntiAliasing` 0x1594cd0
	"bloom": "bloom",	# `GetBloom` 0x15951a0
	"camera_distance": "camera_distance",	# `GetCameraDistance` 0x15951b0
	"casual_matchmaking_region": "casual_matchmaking_region",	# `GetCasualMatchmakingRegion` 0x15952a0
	"character_cloth": "character_cloth",	# `GetCharacterCloth` 0x15952d0
	"character_fidelity": "character_fidelity",	# `GetCharacterFidelity` 0x15952e0
	"combat_headbob": "movement_headbob",	# `GetCombatHeadbob` 0x15952f0
	"crosshair_type": "crosshair_type",	# `GetCrosshairType` 0x1595330
	"crossplay_enabled": "crossplay_enabled",	# `GetCrossplayEnabled` 0x1595340
	"draw_tracers": "draw_tracers",	# `GetDrawTracers` 0x1595bb0
	"effects_volume": "effects_volume",	# `GetEffectsVolume` 0x1595bc0
	"field_of_view": "field_of_view",	# `GetFieldOfView` 0x1595de0
	"force_feedback_enabled": "force_feedback",	# `GetForceFeedbackEnabled` 0x1595e10
	"friendly_markers": "friendly_markers",	# `GetFriendlyMarkers` 0x1595e40
	"gamma": "gamma",	# `GetGamma` 0x15961c0
	"gore": "gore",	# `GetGore` 0x15961d0
	"headbob": "headbob",	# `GetHeadbob` 0x14fc520 (reads +0x18c)
	"hide_default_loadouts": "hide_default_loadouts",	# `GetHideDefaultLoadouts` 0x15961f0
	"hide_hud": "hide_hud",	# `GetHideHUD` 0x1596200
	"indirect_capsule_shadows": "indirect_capsule_shadows",	# `GetIndirectCapsuleShadows` 0x1596210
	"instruments_volume": "instruments_volume",	# `GetInstrumentsVolume` 0x1596220
	"lens_flares": "lens_flares",	# `GetLensFlares` 0xeaf0f0
	"master_volume": "master_volume",	# `GetMasterVolume` 0x1596c80
	"max_ragdolls": "max_ragdolls",	# `GetMaxRagdolls` 0x1596c90
	"motion_blur": "motion_blur",	# `GetMotionBlur` 0x1596cb0
	"mouse_smoothing": "mouse_smoothing",	# `GetMouseSmoothing` 0x1596d00
	"movement_headbob": "movement_headbob",	# `GetMovementHeadbob` 0x15952f0 (same code as GetCombatHeadbob)
	"music_volume": "music_volume",	# `GetMusicVolume` 0x1596d50
	"no_team_colors_on_gear": "no_team_colors_on_gear",	# `GetNoTeamColorsOnGear` 0x1597cc0
	"nvidia_reflex": "nvidia_reflex",	# `GetNvidiaReflex` 0x1597cd0
	"platform_specific": "platform_specific",	# `GetPlatformSpecific` 0x1597d10
	"profanity_filter": "profanity_filter",	# `GetProfanityFilter` 0x1598ac0
	"psn_lock_enabled_value": "is_psn_lock_enabled",	# `GetPSNLockEnabledValue` 0xeaf720
	"ragdoll_fidelity": "ragdoll_fidelity",	# `GetRagdollFidelity` 0x1598ad0
	"ragdoll_stay_time": "ragdoll_stay_time",	# `GetRagdollStayTime` 0x1598ae0
	"ranged_sensitivity": "ranged_sensitivity",	# `GetRangedSensitivity` 0x1598b00
	"ranked_matchmaking_region": "ranked_matchmaking_region",	# `GetRankedMatchmakingRegion` 0x1598bf0
	"screen_percentage": "screen_percentage",	# `GetScreenPercentage` 0x1598d30
	"screen_space_reflections": "screen_space_reflections",	# `GetScreenSpaceReflections` 0x1598d60
	"server_browser_has_players": "server_browser_has_players",	# `GetServerBrowserHasPlayers` 0x15993c0
	"server_browser_is_console_server": "server_browser_console_server",	# `GetServerBrowserIsConsoleServer` 0x15993d0
	"server_browser_is_official": "server_browser_is_official",	# `GetServerBrowserIsOfficial` 0x15993e0
	"server_browser_max_ping": "server_browser_max_ping",	# `GetServerBrowserMaxPing` 0x15993f0
	"server_browser_no_password": "server_browser_no_password",	# `GetServerBrowserNoPassword` 0x1599400
	"server_browser_not_full": "server_browser_not_full",	# `GetServerBrowserNotFull` 0x1599410
	"show_ammo": "show_ammo",	# `GetShowAmmo` 0x1599550
	"show_announcements": "show_announcements",	# `GetShowAnnouncements` 0x1599560
	"show_chat_box": "show_chat_box",	# `GetShowChatBox` 0x1599570
	"show_combat_hints": "show_combat_hints",	# `GetShowCombatHints` 0x1599580
	"show_emotes_menu": "show_emotes_menu",	# `GetShowEmotesMenu` 0x1599590
	"show_equipment": "show_equipment",	# `GetShowEquipment` 0x15995a0
	"show_hit_marker": "show_hit_marker",	# `GetShowHitMarker` 0x15995b0
	"show_kill_feed": "show_kill_feed",	# `GetShowKillFeed` 0x15995c0
	"show_killed_by": "show_killed_by",	# `GetShowKilledBy` 0x15995d0
	"show_objectives": "show_objectives",	# `GetShowObjectives` 0x15995e0
	"show_observed_delay": "show_observed_delay",	# `GetShowObservedDelay` 0x15995f0
	"show_score_feed": "show_score_feed",	# `GetShowScoreFeed` 0x1599600
	"show_server_in_scoreboard": "show_server_in_scoreboard",	# `GetShowServerInScoreboard` 0x1599610
	"show_spawn_info": "show_spawn_info",	# `GetShowSpawnInfo` 0x1599620
	"show_status_bar": "show_status_bar",	# `GetShowStatusBar` 0x1599630
	"show_target_info": "show_target_info",	# `GetShowTargetInfo` 0x1599640
	"show_tips": "show_tips",	# `GetShowTips` 0x1599650
	"third_person_death_camera": "third_person_death_camera",	# `GetThirdPersonDeathCamera` 0x1599cd0
	"tracers_stay_time": "draw_tracers_stay_time",	# `GetTracersStayTime` 0x1599d00
	"video_volume": "video_volume",	# `GetVideoVolume` 0x1599d30
	"voice_volume": "voice_volume",	# `GetVoiceVolume` 0x1599d40
}

const SETTERS := {
	"ambient_occlusion": "ambient_occlusion",	# `SetAmbientOcclusion` 0x15a7100
	"anti_aliasing": "anti_aliasing",	# `SetAntiAliasing` 0x15a7130
	"bloom": "bloom",	# `SetBloom` 0x15a7140
	"camera_distance": "camera_distance",	# `SetCameraDistance` 0x15a7150
	"casual_matchmaking_region": "casual_matchmaking_region",	# `SetCasualMatchmakingRegion` 0x15a7170
	"character_cloth": "character_cloth",	# `SetCharacterCloth` 0x15a7180
	"character_fidelity": "character_fidelity",	# `SetCharacterFidelity` 0x15a7190
	"combat_headbob": "combat_headbob",	# `SetCombatHeadbob` 0x15a71a0
	"crosshair_type": "crosshair_type",	# `SetCrosshairType` 0x15a71c0
	"crossplay_enabled": "crossplay_enabled",	# `SetCrossplayEnabled` 0x15a71d0
	"draw_tracers": "draw_tracers",	# `SetDrawTracers` 0x15a71f0
	"effects_volume": "effects_volume",	# `SetEffectsVolume` 0x15a7200
	"field_of_view": "field_of_view",	# `SetFieldOfView` 0x15a7220
	"force_feedback_enabled": "force_feedback",	# `SetForceFeedbackEnabled` 0x15a7230
	"friendly_markers": "friendly_markers",	# `SetFriendlyMarkers` 0x15a7240
	"gamma": "gamma",	# `SetGamma` 0x15a7300
	"gore": "gore",	# `SetGore` 0x15a7310
	"headbob": "headbob",	# `SetHeadbob` 0x15a7320
	"hide_default_loadouts": "hide_default_loadouts",	# `SetHideDefaultLoadouts` 0x15a7330
	"hide_hud": "hide_hud",	# `SetHideHUD` 0x15a7340
	"indirect_capsule_shadows": "indirect_capsule_shadows",	# `SetIndirectCapsuleShadows` 0x15a7350
	"instruments_volume": "instruments_volume",	# `SetInstrumentsVolume` 0x15a7360
	"lens_flares": "lens_flares",	# `SetLensFlares` 0x15a73c0
	"master_volume": "master_volume",	# `SetMasterVolume` 0x15a73d0
	"max_ragdolls": "max_ragdolls",	# `SetMaxRagdolls` 0x15a7420
	"motion_blur": "motion_blur",	# `SetMotionBlur` 0x15a7430
	"mouse_smoothing": "mouse_smoothing",	# `SetMouseSmoothing` 0x15a7440
	"movement_headbob": "movement_headbob",	# `SetMovementHeadbob` 0x15a7490
	"music_volume": "music_volume",	# `SetMusicVolume` 0x15a74a0
	"no_team_colors_on_gear": "no_team_colors_on_gear",	# `SetNoTeamColorsOnGear` 0x15a74b0
	"nvidia_reflex": "nvidia_reflex",	# `SetNvidiaReflex` 0x15a74c0
	"platform_specific": "platform_specific",	# `SetPlatformSpecific` 0x15a74d0
	"profanity_filter": "profanity_filter",	# `SetProfanityFilter` 0x15a74e0
	"ragdoll_fidelity": "ragdoll_fidelity",	# `SetRagdollFidelity` 0x15a74f0
	"ragdoll_stay_time": "ragdoll_stay_time",	# `SetRagdollStayTime` 0x15a7500
	"ranged_sensitivity": "ranged_sensitivity",	# `SetRangedSensitivity` 0x15a7510
	"ranked_matchmaking_region": "ranked_matchmaking_region",	# `SetRankedMatchmakingRegion` 0x15a7530
	"screen_percentage": "screen_percentage",	# `SetScreenPercentage` 0x15a7610
	"screen_space_reflections": "screen_space_reflections",	# `SetScreenSpaceReflections` 0x15a7620
	"server_browser_has_players": "server_browser_has_players",	# `SetServerBrowserHasPlayers` 0x15a7710
	"server_browser_is_console_server": "server_browser_console_server",	# `SetServerBrowserIsConsoleServer` 0x15a7720
	"server_browser_is_official": "server_browser_is_official",	# `SetServerBrowserIsOfficial` 0x15a7730
	"server_browser_max_ping": "server_browser_max_ping",	# `SetServerBrowserMaxPing` 0x15a7740
	"server_browser_no_password": "server_browser_no_password",	# `SetServerBrowserNoPassword` 0x15a7750
	"server_browser_not_full": "server_browser_not_full",	# `SetServerBrowserNotFull` 0x15a7760
	"show_ammo": "show_ammo",	# `SetShowAmmo` 0x15a77d0
	"show_announcements": "show_announcements",	# `SetShowAnnouncements` 0x15a77e0
	"show_chat_box": "show_chat_box",	# `SetShowChatBox` 0x15a77f0
	"show_combat_hints": "show_combat_hints",	# `SetShowCombatHints` 0x15a7800
	"show_emotes_menu": "show_emotes_menu",	# `SetShowEmotesMenu` 0x15a7810
	"show_equipment": "show_equipment",	# `SetShowEquipment` 0x15a7820
	"show_hit_marker": "show_hit_marker",	# `SetShowHitMarker` 0x15a7830
	"show_kill_feed": "show_kill_feed",	# `SetShowKillFeed` 0x15a7840
	"show_killed_by": "show_killed_by",	# `SetShowKilledBy` 0x15a7850
	"show_objectives": "show_objectives",	# `SetShowObjectives` 0x15a78e0
	"show_observed_delay": "show_observed_delay",	# `SetShowObservedDelay` 0x15a78f0
	"show_score_feed": "show_score_feed",	# `SetShowScoreFeed` 0x15a7900
	"show_server_in_scoreboard": "show_server_in_scoreboard",	# `SetShowServerInScoreboard` 0x15a7910
	"show_spawn_info": "show_spawn_info",	# `SetShowSpawnInfo` 0x15a7920
	"show_status_bar": "show_status_bar",	# `SetShowStatusBar` 0x15a7930
	"show_target_info": "show_target_info",	# `SetShowTargetInfo` 0x15a7940
	"show_tips": "show_tips",	# `SetShowTips` 0x15a7950
	"third_person_death_camera": "third_person_death_camera",	# `SetThirdPersonDeathcamera` 0x15a7c80
	"tracers_stay_time": "draw_tracers_stay_time",	# `SetTracersStayTime` 0x15a8310
	"video_volume": "video_volume",	# `SetVideoVolume` 0x15a8320
	"voice_volume": "voice_volume",	# `SetVoiceVolume` 0x15a8330
}

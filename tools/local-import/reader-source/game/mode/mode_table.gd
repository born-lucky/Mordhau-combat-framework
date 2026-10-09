# mode_table.gd - the ported game modes, one row each: the one match scene (match.tscn, MordhauMatch) reads its row to
# pick the map scene, the mode data and the game mode. The id is the mode's map prefix (UGameMapsSettings
# GameModeMapPrefixes in DefaultEngine.ini; the metadata Prefix the menu filters maps by). Menu order = this order.
#   rooms  the room-based adapter (BP_DuelGameMode and its child BP_Group3v3GameMode): DuelPlayerView reading the
#          controller's ReplicatedRoomGame, the duel HUD, bots logged in like players, dead pawns kept until
#          RestartRoom destroys them (MordhauMatch header)
#   bots   the menu shows the Bot Count slider (hidden for room modes: a room holds MaxPeoplePerRoom)
# Maps: res://data_gen/maps/<prefix>_Arena.tscn (tools/gen_level.gd); SKM_Arena / TF_Arena fall back to a generated
# scene with the same streamed Arena sub-level until the map pipeline builds them (the starts come from the package).
# Frontline (FlMode) is not listed: its map (FL_Camp) is not generated yet.
class_name ModeTable
extends RefCounted

const MATCH_SCENE := "res://game/mode/match.tscn"
const MAPS := "res://data_gen/maps/"

static func rows() -> Array:
	return [
		{"id": "FFA", "map": MAPS + "FFA_Arena.tscn", "fallback": "", "rooms": false, "bots": true,
			"data": func(): return FfaMode.FfaData.shared(), "mode": func(d): return FfaMode.new(d)},
		{"id": "TDM", "map": MAPS + "TDM_Arena.tscn", "fallback": "", "rooms": false, "bots": true,
			"data": func(): return TdmMode.TdmData.shared_tdm(), "mode": func(d): return TdmMode.new(d)},
		{"id": "SKM", "map": MAPS + "SKM_Arena.tscn", "fallback": MAPS + "TDM_Arena.tscn", "rooms": false, "bots": true,
			"data": func(): return SkmMode.SkmData.shared(), "mode": func(d): return SkmMode.new(d)},
		{"id": "DU", "map": MAPS + "DU_Arena.tscn", "fallback": "", "rooms": true, "bots": false,
			"data": func(): return DuelMode.DuelData.shared(), "mode": func(d): return DuelMode.new(d)},
		{"id": "TF", "map": MAPS + "TF_Arena.tscn", "fallback": MAPS + "DU_Arena.tscn", "rooms": true, "bots": false,
			"data": func(): return TfMode.TfData.shared_tf(), "mode": func(d): return TfMode.new(d)},
	]

static func row(id: String) -> Dictionary:
	for r in rows():
		if r.id == id:
			return r
	push_error("ModeTable: no mode %s" % id)
	return {}

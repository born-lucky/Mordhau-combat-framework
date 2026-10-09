# mode_constants.gd - every .rdata literal the game-mode ports (game/mode) take from the exe, loaded once and named
# (same pattern and check as PlayerConstants / CombatConstants: `_k(name, va, functions)`, value = the raw bytes at va,
# every function listed in extract/native/rdata.tsv `funcs`; tests/test_mode.gd runs test_boundary's check_cites).
class_name ModeConstants

static var cites := {}		# name -> [va, PackedStringArray of citing functions, value]

static func _k(name: String, va: int, functions: Array) -> float:
	var v := UeRdata.f32(va)
	cites[name] = [va, PackedStringArray(functions), v]
	return v

const _PREF := "AMordhauGameMode::GetSpawnpointPreference_Implementation"
const _START := "AMordhauPlayerStart::GetSpawnPreferenceFor_Implementation"

# ---- AMordhauGameMode::GetSpawnpointPreference_Implementation rva=0x1599660 -------------------------------------
# per live controller pawn: proximity = max(1000 - distance_cm, 0) * 0.001 (disasm 0x14159978c / 0x141599795)
static var spawn_proximity_range: float = _k("spawn_proximity_range", 0x143fe4e44, [_PREF])
static var spawn_proximity_scale: float = _k("spawn_proximity_scale", 0x144014a8c, [_PREF])
# Preference += clamp(Proximity, -5, 5) - 0.2 * sum(proximity) (0x141599932 / 0x141599942 / 0x14159994a)
static var spawn_proximity_min: float = _k("spawn_proximity_min", 0x144331190, [_PREF])
static var spawn_proximity_max: float = _k("spawn_proximity_max", 0x143fe4e20, [_PREF])
static var spawn_crowd_weight: float = _k("spawn_crowd_weight", 0x1442898a4, [_PREF])
# -10 after the call to `UWorld::EncroachingBlockingGeometry` at 0x141599a75 inside
# AMordhauGameMode::GetSpawnpointPreference_Implementation returns true (load at 0x141599a81); -1000 without a start /
# state / game state (AMordhauGameMode::GetSpawnpointPreference_Implementation 0x141599b9c)
static var spawn_blocked_penalty: float = _k("spawn_blocked_penalty", 0x1443145b0, [_PREF])
static var spawn_invalid: float = _k("spawn_invalid", 0x144352dfc, [_PREF])

# ---- AMordhauPlayerStart::GetSpawnPreferenceFor_Implementation rva=0x15da470 -------------------------------------
# (float)(rand() & 0x7fff) * 1/32767 (disasm 0x1415da486)
static var spawn_rand_scale: float = _k("spawn_rand_scale", 0x1440dfae0, [_START])

# ---- AMordhauGameMode::OnKilled_Implementation rva=0x159d6f0 (assists, team mode) --------------------------------
# a DamageHistory entry counts when now <= its time + 20 (0x14159d96f); fraction = clamp(damage * 0.01, .., 1)
# (0x14159d978 / 0x14159d981); 0.5: the bot "thanks" voice threshold (0x14159d98a)
const _KILL := "AMordhauGameMode::OnKilled_Implementation"
const _TAKE := "AMordhauCharacter::TakeDamage"
static var assist_window: float = _k("assist_window", 0x143fe4e30, [_KILL, _TAKE])
static var assist_damage_scale: float = _k("assist_damage_scale", 0x144014a94, [_KILL])
static var assist_fraction_max: float = _k("assist_fraction_max", 0x143fe4e0c, [_KILL])
static var assist_thanks_fraction: float = _k("assist_thanks_fraction", 0x143fe4e04, [_KILL])
# AMordhauGameMode::RequestedAssignTeam_Implementation rva=0x15a6070: start team = (rand() & 0x7fff) * 1/32767 * TeamCount
static var team_rand_scale: float = _k("team_rand_scale", 0x1440dfae0, ["AMordhauGameMode::RequestedAssignTeam_Implementation"])

# bot_duel.gd - runs bots inside a CombatState (godot/game/combat): the combat step for time t, then every bot's
# AI frame at t, reading the fighters' motions as they stand after that step. This is the reference hookup the
# duel scene can copy (README.md). Order inside a frame: UE ticks the AI controller / BT component and the character's
# motion component in tick groups whose relative order is not taken from decompiled code (UNCONFIRMED).
class_name BotDuel
extends RefCounted

var world: CombatState
var bodies: Array = []			# BotBody, every fighter
var bots: Array = []			# MordhauBotController
var rng := UeRand.new()			# one CRT rand() stream for the whole world (UeRand header)
var world_state := {}
var team_mode := false			# GameState bIsTeamMode for every bot's perception

func _init(w: CombatState) -> void:
	world = w

# wrap an existing fighter (CombatState.add_fighter) as a body at a UE-space position / yaw
func add_body(nm: String, location: Vector3, yaw: float, weapon_length: float, team := 0) -> BotBody:
	var b := BotBody.new(nm, world.fighters[nm])
	b.max_walk_speed = BotData.max_walk_speed()
	b.location = location
	b.yaw = yaw
	b.weapon_length = weapon_length
	b.team = team
	bodies.append(b)
	return b

func add_bot(b: BotBody, behavior := "BOTBEHAVIOR_Knight", tree := MordhauBotController.DEFAULT_TREE) -> MordhauBotController:
	var c := MordhauBotController.make(b, bodies, behavior, rng, tree)
	c.combat = world
	c.world_state = world_state
	c.team_mode = team_mode
	bots.append(c)
	for o in bots:				# every bot's perception sees the bodies present now (a fresh entry refreshes at once)
		o.update_perception()
	return c

func step() -> void:
	world.step()
	for c in bots:
		c.tick(world.dt)

func run_until(t_end: float) -> void:
	while world.now < t_end - world.dt * 0.5:
		step()

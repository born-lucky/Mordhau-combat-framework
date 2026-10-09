# bt_voice_or_emote.gd - UBTTask_VoiceOrEmote, ported from extract/native/decomp/UBTTask_VoiceOrEmote.cpp
#   ExecuteTask rva=0x147fed0; parameters BotData.BtTaskParams (ctor rva=0x144dd00 replayed by NativeCtor: GlobalCooldown
#   2.0, Chance 0.1, lists empty, bForceEmote 0; the node values of the BehaviorTree package over them).
# Instant task: one rand() against Chance first (fails the task when above it); then, cooldowns allowing, a random
# entry of VoiceCommandsList / EmotesList. The voice/emote itself is presentation (not in the combat port): it is
# emitted as an event for the scene. The cooldown timestamps live on the game mode (+0x770 voice, +0x774 emote) in
# the game; here on the shared `world_state` dictionary the controllers of one duel share.
class_name BtVoiceOrEmote
extends BtTask

const FN := "UBTTask_VoiceOrEmote::ExecuteTask rva=0x147fed0"

func execute(c) -> int:
	var chance := params.chance
	var r: float = c.rng.frand()
	if not (r <= chance):
		return FAILED
	var me: BotBody = c.body
	if me == null or me.is_dead:
		return FAILED
	var now: float = c.now()
	var gcd := params.global_cooldown
	var ws: Dictionary = c.world_state
	var voices := params.voice_commands
	if voices.size() > 0:
		var i := mini(int(float(c.rng.rand()) * K.voice_pick_rand_scale * float(voices.size())), voices.size() - 1)
		# game mode +0x770 last voice time; voice component +0xe8 last voice time + 3.0 (_DAT_143fe4e10)
		if gcd + float(ws.get("last_voice", -INF)) < now and float(c.last_voice_time) + K.voice_min_interval < now:
			ws["last_voice"] = now
			c.last_voice_time = now
			c.events.append({"t": now, "kind": "voice", "id": int(voices[i])})	# RequestVoiceCommand(id, 0)
			c.note("VoiceOrEmote", "voice %d" % int(voices[i]), FN)
	var emotes := params.emotes
	if emotes.size() > 0:
		var j := mini(int(float(c.rng.rand()) * K.voice_pick_rand_scale * float(emotes.size())), emotes.size() - 1)
		if gcd + float(ws.get("last_emote", -INF)) < now:
			ws["last_emote"] = now
			# bForceEmote: AssignNetMotion {MotionType 0x0a, id}; else UMotionSystemComponent::RequestEmote
			c.events.append({"t": now, "kind": "emote", "id": int(emotes[j]), "forced": params.b_force_emote})
			c.note("VoiceOrEmote", "emote %d" % int(emotes[j]), FN)
	return SUCCEEDED

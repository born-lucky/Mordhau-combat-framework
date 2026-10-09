# bt_fall_for_feint.gd - UBTTask_FallForFeint, ported from extract/native/decomp/UBTTask_FallForFeint.cpp
#   PerformFeintEvaluation rva=0x1495720 (ExecuteTask rva=0x147fe50 jumps to it; TickTask rva=0x14a50e0 finishes the
#   latent task on anything but InProgress).
# A bot that rolled bWillFallForFeint parries a feint or a morph of the enemy it is facing: a morph at once, a feint
# 0.2 s after the feint began; while a real attack is still more than 0.1 s from WindupEnd it keeps watching.
class_name BtFallForFeint
extends BtTask

const FN := "UBTTask_FallForFeint::PerformFeintEvaluation rva=0x1495720"

func execute(c) -> int:
	return perform(c)

func tick(c, _dt: float) -> int:
	return perform(c)

func perform(c) -> int:
	var me: BotBody = c.body
	if me == null or me.sys == null:
		return FAILED
	var prof: BotBehaviorProfile = c.profile
	c.get_motion_based_random()
	if not prof.p.will_fall_for_feint:					# +0xd4
		return SUCCEEDED
	var now: float = c.now()
	var my_loc := me.location
	var watching := false									# cVar7
	for en: BotBody in c.perceived_enemies():
		if c.currently_facing_actor() != en:				# GetCurrentlyFacingActor rva=0x14fafd0 (+0x33c)
			continue
		var d := safe_normal(my_loc - en.location)
		if d.dot(en.forward()) < K.feint_enemy_facing_dot:	# -0.5: enemy not facing me
			continue
		var m = en.sys.motion if en.sys != null else null
		var parry_now := false								# bVar5
		var wait := false									# bVar6
		if m is FeintedMotion:								# UFeintedMotion StaticClass 0x14168e7c0
			var t: float = m.start_time + K.feint_react_delay	# StartTime (+0x4c) + 0.2
			parry_now = t < now
			wait = now <= t
		elif m is AttackMotion:
			if m.type == CombatEnums.AttackType.MORPH:		# Type (+0x108d) == 4
				parry_now = true
			else:
				if m.windup_end - K.feint_watch_lead <= now:	# WindupEnd - 0.1
					continue
				wait = true
		else:
			continue
		if (en.location - my_loc).length_squared() <= K.feint_range_sq:	# 32400 = 180^2
			if parry_now:
				c.note("FallForFeint", "%s %s -> RequestParry(Regular), Succeeded" % [en.name, m.kind()], FN)
				c.request_parry(CombatEnums.BlockType.REGULAR)	# RequestParry(0, true)
				return SUCCEEDED
			if wait:
				watching = true
	if watching:
		c.note("FallForFeint", "watching a windup / fresh feint -> InProgress", FN)
		return IN_PROGRESS
	return SUCCEEDED

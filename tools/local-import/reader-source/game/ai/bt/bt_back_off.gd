# bt_back_off.gd - BTTask_BackOff_C (Mordhau/Content/Mordhau/AI/Tasks/BTTask_BackOff), the combat tree's spacing /
# circling move, ported from its Blueprint bytecode (scripts/kismet; statement indices in brackets; literals:
# ModeKismet "ai_backoff_*"). Blueprint variables (none in the CDO: 0 / false): RequiresInitialization, OriginalAngle,
# RightOffset, RandomSide, Distance. A Blueprint task (UBTTask_BlueprintBase) stays InProgress until FinishExecute.
#   ReceiveExecuteAI [1997]: only when RequiresInitialization: clear it; with a closest enemy: OriginalAngle = yaw of
#     MakeRotFromZX(Up, Normal(pawn - enemy, 1e-4)), RightOffset = 0, RandomSide = RandomFloat > 0.5 ? 1 : -1
#   ReceiveTickAI [27]: closest enemy, and (GetMotionBasedRandom < 0.1 or bIsClosestEnemySaturated), else [1774]
#     RequiresInitialization = true, FinishExecute(true). Weaker than the enemy (our Health < byte(round(enemy Health *
#     0.5)) or our Stamina < byte(round(enemy Stamina * 0.5))): Distance = RandomFloat * 300 + 700, StopSprinting;
#     else only when saturated: Distance = RandomFloat * 500 + 200, StartSprinting (not saturated -> [1774]).
#     Then [1115]: RightOffset += dt * RandomSide; MoveToLocation(enemy + rotate((1,0,0), yaw OriginalAngle +
#     RightOffset * 360 * 0.33) * 700, acceptance -1); StartFacingActor(enemy, 25, (0,0)); FinishExecute(false).
#     Distance is computed and never read (the move radius is the literal 700).
class_name BtBackOff
extends BtTask

var requires_initialization := false
var original_angle := 0.0
var right_offset := 0.0
var random_side := 0.0
var distance := 0.0

static func _kv(k: String):
	return ModeKismet.kv(k)

func execute(c) -> int:
	if requires_initialization:
		requires_initialization = false
		var en: BotBody = c.closest_enemy()
		if en != null:
			var away: Vector3 = c.body.location - en.location
			# UKismetMathLibrary::Normal(v, 1e-4) = FVector::GetSafeNormal: zero when |v|^2 < tolerance
			var n := away.normalized() if away.length_squared() >= float(_kv("ai_backoff_normal_tol")) else Vector3.ZERO
			original_angle = rad_to_deg(atan2(n.y, n.x))			# MakeRotFromZX(Up, n): yaw of n's XY direction
			right_offset = 0.0
			random_side = 1.0 if c.random_float > float(_kv("ai_backoff_side_split")) else -1.0
	return IN_PROGRESS

func tick(c, dt: float) -> int:
	var en: BotBody = c.closest_enemy()
	var me: BotBody = c.body
	if en == null or me == null or me.sys == null or not (c.get_motion_based_random() < float(_kv("ai_backoff_motion_random")) \
			or c.closest_enemy_saturated):
		return _done(c, true)
	var frac := float(_kv("ai_backoff_enemy_frac"))
	var weaker: bool = me.health_byte() < (floori(float(en.health_byte()) * frac + 0.5) & 0xff) \
		or me.stamina_byte() < (floori(float(en.stamina_byte()) * frac + 0.5) & 0xff)
	if weaker:
		distance = c.random_float * float(_kv("ai_backoff_weak_rand")) + float(_kv("ai_backoff_weak_base"))
		me.wants_sprint = false								# StopSprinting
	elif c.closest_enemy_saturated:
		distance = c.random_float * float(_kv("ai_backoff_sat_rand")) + float(_kv("ai_backoff_sat_base"))
		c.start_sprinting()
	else:
		return _done(c, true)
	right_offset += dt * random_side
	var yaw := deg_to_rad(original_angle + right_offset * float(_kv("ai_backoff_turn_deg")) * float(_kv("ai_backoff_turn_scale")))
	var dest: Vector3 = en.location + Vector3(cos(yaw), sin(yaw), 0.0) * float(_kv("ai_backoff_radius"))
	c.move_to_location(dest, float(_kv("ai_backoff_acceptance")))
	c.start_facing_actor(en, float(_kv("ai_backoff_face_up")), Vector2.ZERO)
	c.note("BackOff", "circle to yaw %.1f (distance %.0f, %s)" % [rad_to_deg(yaw), distance, "weaker" if weaker else "saturated"],
		"BTTask_BackOff_C ReceiveTickAI")
	return FAILED												# FinishExecute(false)

func _done(c, ok: bool) -> int:
	requires_initialization = true
	c.note("BackOff", "no back-off -> %s" % ("Succeeded" if ok else "Failed"), "BTTask_BackOff_C ReceiveTickAI")
	return SUCCEEDED if ok else FAILED

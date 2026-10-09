# weapon_tracer.gd - the melee weapon's hit trace: AMordhauWeapon's tracer state and its sampling.
# Sources (extract/native/decomp/AMordhauWeapon.cpp, field names from types/AMordhauWeapon.h):
#   PrepareForTracing rva=0x16378e0: Previous{Start,End} = Current{Start,End}; Current = sockets now (GetTrace);
#     bArePreviousTracersValid = bAreCurrentTracersValid; bAreCurrentTracersValid = !bAreCurrentTracersInvalidated
#     (then the invalidated flag is cleared). It runs from UAttackMotion::OnLateTick_Implementation rva=0x1631f00
#     (weapon vtable +0x7c0) every late tick while an attack motion is current.
#   ResetTracers rva=0x163bb80: bAreCurrentTracersInvalidated = 1.
#   GetTrace_Implementation rva=0x1629520: Current = mesh socket "TraceStart" / "TraceEnd" (Second* in alt mode).
#   SampleTracers rva=0x163c430 (non-cosmetic path, param_2 = 0), only when both tracer sets are valid:
#     dirC = normalize(CurStart - CurEnd), dirP = normalize(PrevStart - PrevEnd)
#     n = RoundToInt(|CurEnd - CurStart| * 0.26667 / 2) (exe: ROUND(len * tracer_count_scale + 0.5) >> 1, the SSE
#       RoundToInt; tracer_count_scale = 2/7.5)
#     for i = n .. 0 (tracer_step = -1): segment from PrevEnd + dirP * i * 7.5 to CurEnd + dirC * i * 7.5
#       (tracer_spacing_cm = 7.5), i.e. sample points every 7.5 cm from TraceStart side (i = n) to TraceEnd (i = 0), each swept from
#       where it was last tick to where it is now -> SampleTracer(prev point, cur point).
#   Not ported: the 4 extra environment tracers past the base (bUsesExtraEnvironmentTracers, +0x1a18; their hits
#     only count against weapons, SampleTracer param_7), additional tracers (shields), cosmetic tracers.
# Positions are Godot world space (m); 7.5 cm -> 0.075 m by the same x0.01 the glTF writer uses.
class_name WeaponTracer
extends RefCounted

const CM_PER_M := 100.0		# sockets are stored in Godot metres (glTF writer: UE cm x 0.01); UE math is in cm

var trace_start_socket: UePhysics.Socket = null	# null = the mesh has no such socket
var trace_end_socket: UePhysics.Socket = null
var cur_start := Vector3.ZERO	# +0xd48 CurrentTraceStart
var cur_end := Vector3.ZERO		# +0xd54 CurrentTraceEnd
var prev_start := Vector3.ZERO	# +0xd60 PreviousTraceStart
var prev_end := Vector3.ZERO	# +0xd6c PreviousTraceEnd
var b_cur_valid := false		# +0xd40 bAreCurrentTracersValid
var b_prev_valid := false		# +0xd41 bArePreviousTracersValid
var b_cur_invalidated := false	# +0xd42 bAreCurrentTracersInvalidated
var root_xf := Transform3D.IDENTITY	# world transform of the socket bone, fed each tick by the game
var actor_ignore_cache := []	# MotionSystem.id of each actor; +0xe88 ActorIgnoreCache: UAttackMotion::OnBegin_Implementation rva=0x162eda0 empties it
								# and adds the owner; ProcessHitForDamage rva=0x1638a60 appends every damaged actor
var length := 0.0				# +0x1bec Length (UE units / 15, see _init)

func _init(w: WeaponData) -> void:
	var mesh := UePhysics.weapon_mesh(w) if w != null else ""
	trace_start_socket = UePhysics.socket(mesh, UePhysics.TRACE_START)	# mesh "" = none (BP_FistsWeapon)
	trace_end_socket = UePhysics.socket(mesh, UePhysics.TRACE_END)
	# AMordhauWeapon::RecalculateTracerPoints rva=0x163a940: Length = |TraceEnd.Z - TraceStart.Z| (component space,
	# UE cm) x CombatConstants.tracer_length_per_cm (1/15). UE Z is Godot Y; sockets are stored in metres, so x100 back to cm.
	if has_sockets():
		var span_m := absf(trace_end_socket.loc.y - trace_start_socket.loc.y)
		var span_cm := span_m * CM_PER_M
		length = span_cm * CombatConstants.tracer_length_per_cm

func has_sockets() -> bool:
	return trace_start_socket != null and trace_end_socket != null

# socket position in world space for the current root transform (sockets on the weapon root bone "Armature")
func socket_world(s: UePhysics.Socket) -> Vector3:
	return root_xf * (s.loc if s != null else Vector3.ZERO)

# from AMordhauWeapon::ResetTracers rva=0x163bb80
func reset_tracers() -> void:
	b_cur_invalidated = true

# from AMordhauWeapon::PrepareForTracing rva=0x16378e0
func prepare_for_tracing() -> void:
	prev_start = cur_start
	prev_end = cur_end
	cur_start = socket_world(trace_start_socket)
	cur_end = socket_world(trace_end_socket)
	var was_invalidated := b_cur_invalidated
	b_prev_valid = b_cur_valid
	if was_invalidated:
		b_cur_invalidated = false
	b_cur_valid = not was_invalidated

# from AMordhauWeapon::SampleTracers rva=0x163c430: [[from, to], ...], i = n (base side) first, TraceEnd last
func segments() -> Array:
	if not (b_cur_valid and b_prev_valid):
		return []
	var dir_c := (cur_start - cur_end).normalized()
	var dir_p := (prev_start - prev_end).normalized()
	var len_cm := (cur_end - cur_start).length() * 100.0
	var n := UeMath.cvtss2si(len_cm * CombatConstants.tracer_count_scale + CombatConstants.tracer_round_bias) >> 1
	var step := CombatConstants.tracer_spacing_cm * 0.01		# 7.5 cm in metres
	var out := []
	var i := float(n)
	while i >= 0.0:
		out.append([prev_end + dir_p * i * step, cur_end + dir_c * i * step])
		i += CombatConstants.tracer_step
	return out

# ue_math.gd - the float -> int conversions and small range helpers the decompiled combat code inlines, as plain
# functions (no state). Each one names the instruction sequence or function it reproduces; the shared constants
# (0.5 bias, SMALL_NUMBER) come from CombatConstants like every other exe literal.
class_name UeMath

# x86 cvtss2si (Ghidra's ROUND): round half to even under the default MXCSR (round-to-nearest-even).
static func cvtss2si(v: float) -> int:
	var f := floorf(v)
	var d := v - f
	if d > 0.5:
		return int(f) + 1
	if d < 0.5:
		return int(f)
	return int(f) + (int(f) & 1)

# x86 cvttss2si (Ghidra's (int) cast of a float): truncation toward zero.
static func cvttss2si(v: float) -> int:
	return int(v)

# FMath::RoundToInt as UE 4.26 compiles it on SSE: cvtss2si(2x + 0.5) >> 1 (seen in AMordhauWeapon::SampleTracers
# rva=0x163c430 and UStatComponent::WriteReplicatedStat rva=0x1520b50); callers pass the already doubled+biased value
# to cvtss2si where the exe precomputes it, this is the plain form.
static func round_to_int(v: float) -> int:
	return cvtss2si(v + v + 0.5) >> 1

# from UMordhauUtilityLibrary::GetNormalizedTime rva=0x1624620:
#   Current < End: Current <= Start -> 0; Start < End -> (Current - Start) / (End - Start); otherwise 1
static func normalized_time(start: float, end: float, current: float) -> float:
	if current < end:
		if current <= start:
			return 0.0
		if start < end:
			return (current - start) / (end - start)
	return 1.0

# The range fraction UBlockedMotion::OnBegin_Implementation rva=0x165dc20 (stab stop fade, decomp lines 263-273) and
# OnTick_Implementation rva=0x16671b0 (procedural bounce, lines 584-599) inline:
#   x' = clamp(x, lo, hi) (lo first: x < lo -> lo, else hi <= x -> hi)
#   |lo - hi| > SMALL_NUMBER and lo <= hi -> clamp((x' - lo) / (hi - lo), 0, 1), otherwise 0
static func range_fraction(lo: float, hi: float, x: float) -> float:
	var c := lo
	if lo <= x:
		c = x
		if hi <= x:
			c = hi
	if absf(lo - hi) <= CombatConstants.small_number or hi < lo:
		return 0.0
	return clampf((c - lo) / (hi - lo), 0.0, 1.0)

# round to IEEE-754 binary32: the game computes in float (e.g. 32767 * (1/32767f) is exactly 1.0f, while the same
# product in double is 0.999999999). Engine-neutral meaning; the Godot implementation is a packed float32 store.
# (Was game/ai/bot_k.gd BotK.f32; the bot code and UeRand use it.)
static func f32(x: float) -> float:
	return PackedFloat32Array([x])[0]

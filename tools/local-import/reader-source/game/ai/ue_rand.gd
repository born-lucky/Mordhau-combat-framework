# ue_rand.gd - the random source every bot function draws from.
# Mordhau's bot code calls the C runtime rand() directly and scales it itself:
#   (float)(rand() & 0x7fff) * _DAT_1440dfae0      with _DAT_1440dfae0 = 1/32767 (3.0518509e-05, extract/native/rdata.tsv)
# e.g. UBotBehaviorProfile::RerollRandomInstanceValues rva=0x149e910, UBTTask_MeleeAttack::PerformOffensiveEvaluation
# rva=0x1495d00 (disasm 0x141496acc: `call [rip+0x2b123ae]` = the CRT import, then `and eax, 0x7fff`).
# UE's own UBTService::ScheduleNextTick rva=0x38b8190 and UBTTask_Wait::ExecuteTask rva=0x38a5d40 draw from the same
# import (disasm: `call [rip+...]` -> [143fa8e80]), so one stream is shared by every caller in a world.
#
# The generator is the documented Microsoft CRT rand(): seed = seed * 214013 + 2531011, return (seed >> 16) & 0x7fff,
# initial seed 1 (https://learn.microsoft.com/cpp/c-runtime-library/reference/rand: "rand ... generates a well-known
# sequence"; srand: "rand behaves as if srand(1) had been called"). UNCONFIRMED: the shipped UCRT's constants were not
# disassembled (the import lives in ucrtbase.dll, not in the game exe). Engine code that also calls rand() every frame
# (perception, animation, other actors) is not simulated, so the stream position in a real match is not reproduced;
# only the formulas applied to each draw are.
#
# For tests, `forced` holds raw rand() results to return first (exact branch control).
class_name UeRand
extends RefCounted

var seed := 1
var calls := 0
var forced: Array = []

func _init(s := 1) -> void:
	seed = s

func rand() -> int:
	calls += 1
	if not forced.is_empty():
		return int(forced.pop_front()) & 0x7fff
	seed = (seed * 214013 + 2531011) & 0xffffffff
	return (seed >> 16) & 0x7fff

# FMath::FRand as the game inlines it: (rand() & 0x7fff) * (1/32767)
func frand() -> float:
	return UeMath.f32(float(rand()) * BotConstants.frand_scale)

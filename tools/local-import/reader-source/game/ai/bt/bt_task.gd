# bt_task.gd - base of every behavior-tree task the bot trees use (UBTTaskNode).
# Results are EBTNodeResult values, the same integers the game's tasks return in the Ghidra C:
# Succeeded 0, Failed 1, Aborted 2, InProgress 3 (e.g. UBTTask_MeleeDefend::AbortTask rva=0x1456e80 returns 2,
# TickTask rva=0x14a5160 keeps the task latent while PerformDefensiveEvaluation returns 3).
#
# execute = ExecuteTask, tick = TickTask (only while latent; a result other than InProgress is FinishLatentTask),
# abort = AbortTask. `params` is the node's typed parameters (BotData.BtTaskParams, from the BehaviorTree package).
class_name BtTask
extends RefCounted

const SUCCEEDED := 0
const FAILED := 1
const ABORTED := 2
const IN_PROGRESS := 3
const K = preload("res://components/ue/rdata/bot_constants.gd")	# BotConstants: named .rdata literals
const RESULT_NAMES := ["Succeeded", "Failed", "Aborted", "InProgress"]

var node_name := ""
var type_name := ""
var params := BotData.BtTaskParams.new()

func setup(nm: String, ty: String, p: BotData.BtTaskParams) -> BtTask:
	node_name = nm
	type_name = ty
	params = p
	return self

func execute(_c) -> int:
	return SUCCEEDED

func tick(_c, _dt: float) -> int:
	return IN_PROGRESS

func abort(_c) -> int:
	return ABORTED

# the safe normal UE's FVector::GetSafeNormal inlines in every task (rsqrt + two Newton steps; exact 1.0 kept,
# squared length < 1e-8 -> zero vector): _DAT_143fe4e0c = 1.0, _DAT_144014a88 = 1e-8
static func safe_normal(v: Vector3) -> Vector3:
	var l2 := v.length_squared()
	if l2 == 1.0:
		return v
	if l2 < K.safe_normal_min_sq:
		return Vector3.ZERO
	return v / sqrt(l2)

# mordhau_anim.gd - locomotion AnimationTree for the UMA body, from Mordhau's own blend spaces.
# The real graph is AB_MordhauCharacterAnimation (an AnimBlueprint; not ported). What is taken from data:
#  - lower body: BS_LowerBodyLocomotion_Front (Direction -180..180, Velocity 0..1 = walk..run), its sample points
#    read from extract/json; the clips are the AnimSequences those samples name (exported with `mdx anim`).
#  - upper body: HumanMeshComponent.UnarmedUpperBlendSpace = BS_Unarmed_locomotion_3P_V2 (Direction, Velocity 0..90).
#  - idle: HumanMeshComponent.UnarmedLowerAnimation = Unarmed_Idle_3p_New_Still; falling: UnarmedFallingAnimation =
#    Airborne (BP_MordhauCharacter.json, export CharacterMesh0).
# Clips play through UeAnim (godot/components/ue/ue_anim.gd). Upper/lower split at the "Spine" bone is our choice.
class_name MordhauAnim

const RAW := "Mordhau/Content/Mordhau/Animations/RawClips/"
const BS_LOWER := "Mordhau/Content/Mordhau/Animations/BlendSpaces/LowerBody/BS_LowerBodyLocomotion_Front"
const BS_UPPER := "Mordhau/Content/Mordhau/Animations/BlendSpaces/UpperBody/BS_Unarmed_locomotion_3P_V2"
const CROUCH_IDLE := RAW + "Locomotion/Idle_Crouched_L"
const CROUCH_WALK := RAW + "Locomotion/WalkForwardLeft_Crouched"
const UPPER_SPLIT_BONE := "Spine"

# the samples of a blend space whose clip can be loaded (UeAnim.has_asset: in the paks, or exported as a glb)
static func samples(bs_path: String) -> Array[AnimData.Sample]:
	var out: Array[AnimData.Sample] = []
	for s in AnimData.blend_space(bs_path).samples:
		if s.seq != "" and UeAnim.has_asset(s.seq):
			out.append(s)
	return out

static func axis_max(bs_path: String, i: int) -> float:
	var m := AnimData.blend_space(bs_path).max_v
	return m.x if i == 0 else m.y

# Builds AnimationPlayer + AnimationTree under `body` (an instance of the UMA_Master glb). Returns the tree; drive it
# with set_params().
static func build(body: Node3D) -> AnimationTree:
	var mp := CharacterData.mesh()
	var idle := mp.unarmed_lower_animation
	var fall := mp.unarmed_falling_animation
	var lower := samples(BS_LOWER)
	var upper := samples(BS_UPPER)
	var paths := [idle, fall, CROUCH_IDLE, CROUCH_WALK]
	for s in lower + upper:
		if not s.seq in paths: paths.append(s.seq)
	paths = paths.filter(func(p): return UeAnim.has_asset(p))
	var skp := UeAnim.skel_path(body)
	var lib := UeAnim.library(paths, skp)
	for n in lib.get_animation_list():
		lib.get_animation(n).loop_mode = Animation.LOOP_LINEAR
	var ap := UeAnim.attach(body, lib)
	var tree := AnimationTree.new()
	tree.name = "LocomotionTree"
	body.add_child(tree)
	tree.root_node = NodePath("..")
	tree.anim_player = tree.get_path_to(ap)
	var bt := AnimationNodeBlendTree.new()

	var lo := AnimationNodeBlendSpace2D.new()
	lo.min_space = Vector2(-180, 0); lo.max_space = Vector2(180, axis_max(BS_LOWER, 1))
	for s in lower: lo.add_blend_point(_clip(s.seq), s.value, -1, StringName(("%s_%d_%d" % [s.seq.get_file(), s.value.x, s.value.y]).replace("-", "_")))
	var up := AnimationNodeBlendSpace2D.new()
	up.min_space = Vector2(-180, 0); up.max_space = Vector2(180, axis_max(BS_UPPER, 1))
	for s in upper: up.add_blend_point(_clip(s.seq), s.value, -1, StringName(("%s_%d_%d" % [s.seq.get_file(), s.value.x, s.value.y]).replace("-", "_")))
	bt.add_node("lower", lo, Vector2(0, 0))
	bt.add_node("upper", up, Vector2(0, 200))
	bt.add_node("idle", _clip(idle), Vector2(0, 400))
	bt.add_node("crouch_idle", _clip(CROUCH_IDLE), Vector2(0, 500))
	bt.add_node("crouch_walk", _clip(CROUCH_WALK), Vector2(0, 600))
	bt.add_node("fall", _clip(fall), Vector2(0, 700))

	# upper body layered over lower body from UPPER_SPLIT_BONE down (Blend2 with a bone filter)
	var layer := AnimationNodeBlend2.new()
	layer.filter_enabled = true
	var sk := UeAnim.skeleton_of(body)
	var split := sk.find_bone(UPPER_SPLIT_BONE)
	for i in sk.get_bone_count():
		var b := i
		while b >= 0 and b != split: b = sk.get_bone_parent(b)
		if b == split: layer.set_filter_path(NodePath(String(skp) + ":" + sk.get_bone_name(i)), true)
	bt.add_node("layer", layer, Vector2(300, 100))
	bt.connect_node("layer", 0, "lower"); bt.connect_node("layer", 1, "upper")
	var moving := AnimationNodeBlend2.new()            # idle <-> moving by speed
	bt.add_node("moving", moving, Vector2(500, 200))
	bt.connect_node("moving", 0, "idle"); bt.connect_node("moving", 1, "layer")
	var cw := AnimationNodeBlend2.new()
	bt.add_node("crouch_move", cw, Vector2(500, 500))
	bt.connect_node("crouch_move", 0, "crouch_idle"); bt.connect_node("crouch_move", 1, "crouch_walk")
	var crouch := AnimationNodeBlend2.new()
	bt.add_node("crouch", crouch, Vector2(700, 300))
	bt.connect_node("crouch", 0, "moving"); bt.connect_node("crouch", 1, "crouch_move")
	var air := AnimationNodeBlend2.new()
	bt.add_node("air", air, Vector2(900, 300))
	bt.connect_node("air", 0, "crouch"); bt.connect_node("air", 1, "fall")
	bt.connect_node("output", 0, "air")
	tree.tree_root = bt
	tree.active = true
	set_params(tree, 0.0, 0.0, 0.0, 0.0, false, false)
	return tree

static func _clip(p: String) -> AnimationNodeAnimation:
	var a := AnimationNodeAnimation.new()
	a.animation = "ue/" + p.get_file()
	return a

# direction: UE CalculateDirection, degrees, + = moving to the right of the facing. walk_run: 0 walk .. 1 run
# (lower blend space Velocity). upper_vel: BS_Unarmed Velocity axis value. moving: 0..1 idle->move weight.
static func set_params(t: AnimationTree, direction: float, walk_run: float, upper_vel: float, moving: float,
		crouched: bool, falling: bool) -> void:
	t.set("parameters/lower/blend_position", Vector2(direction, walk_run))
	t.set("parameters/upper/blend_position", Vector2(direction, upper_vel))
	t.set("parameters/moving/blend_amount", moving)
	t.set("parameters/crouch_move/blend_amount", moving)
	t.set("parameters/crouch/blend_amount", 1.0 if crouched else 0.0)
	t.set("parameters/air/blend_amount", 1.0 if falling else 0.0)

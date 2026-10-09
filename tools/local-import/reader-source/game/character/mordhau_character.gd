# mordhau_character.gd - the player body: AMordhauCharacter's capsule + mesh, driven by MordhauMovement.
# CharacterBody3D (docs.godotengine.org/en/stable/classes/class_characterbody3d.html) only does collision: the model
# computes velocity in UE cm/s and the body moves by it with move_and_slide() in m/s (x0.01, docs/DESIGN.md section 4).
# Actor forward is Godot -Z (camera forward); the UE frame (+X forward) maps to it by +90 deg about Y.
class_name MordhauCharacter
extends CharacterBody3D

const UMA_MESH := "Mordhau/Content/UMA/UMA/Master/UMA_Master"
const CM := 0.01

# AMordhauCharacter::AMordhauCharacter rva=0x1524d90 writes the CapsuleComponent (ACharacter +0x290):
# +0x458 CapsuleHalfHeight = 0x42c00000 (96), +0x45c CapsuleRadius = 0x42480000 (50). Field order checked against
# the engine ACharacter ctor, which writes InitCapsuleSize(34, 88) as +0x45c = 34, +0x458 = 88.
const CAPSULE_HALF_HEIGHT := 96.0
const CAPSULE_RADIUS := 50.0

var move: MordhauMovement
# Set before add_child: loadout -> armor speed factors (MordhauMovement.apply_loadout) and the mesh.
# build_mesh: optional Callable(Loadout) -> Node3D holding the UMA skeleton (e.g. CharacterBuilder.build); default
# is CharacterBuilder.build for a loadout, the bare UMA_Master otherwise.
var loadout: Loadout
var build_mesh: Callable
var mesh_root: Node3D        # UMA_Master instance
var ue_frame: Node3D         # UE actor frame (+X forward) under the Godot body (-Z forward)
var anim: AnimationTree
var shape: CollisionShape3D
var yaw := 0.0               # control rotation, degrees
var pitch := 0.0
var look_up_limit := 0.0     # CharacterData.Character.look_up_limit (BP_MordhauCharacter CDO LookUpLimit)
var look_down_limit := 0.0   # LookDownLimit
var mouse := {}
var crouched_half_height := 65.0
var _crouch_held := false    # last frame's Crouch / Sprint buttons (pressed / released edges -> MordhauMovement)
var _sprint_held := false

static func ue_to_godot_speed(cm_per_s: float) -> float:
	return cm_per_s * CM

func _ready() -> void:
	move = MordhauMovement.load_default()
	crouched_half_height = move.cfg.crouched_half_height
	look_up_limit = CharacterData.character().look_up_limit
	look_down_limit = CharacterData.character().look_down_limit
	# walkable floor + step height from CharMoveComp (WalkableFloorZ is cos of the max angle)
	floor_max_angle = acos(move.cfg.walkable_floor_z)
	floor_snap_length = move.cfg.max_step_height * CM
	shape = CollisionShape3D.new()
	shape.shape = CapsuleShape3D.new()
	add_child(shape)
	_set_half_height(CAPSULE_HALF_HEIGHT)
	ue_frame = Node3D.new()
	ue_frame.name = "UeFrame"
	ue_frame.rotation_degrees.y = 90.0
	add_child(ue_frame)
	if loadout != null:
		move.apply_loadout(loadout)
	mesh_root = build_mesh.call(loadout) if build_mesh.is_valid() else default_mesh(loadout)
	if mesh_root != null:
		ue_frame.add_child(mesh_root)
		_place_mesh(CAPSULE_HALF_HEIGHT)
		anim = MordhauAnim.build(mesh_root)

# CharacterMesh0 RelativeLocation (0,0,-97) / RelativeRotation yaw -90 (BP_MordhauCharacter.json). In UE the mesh's
# relative Z follows the capsule bottom when crouching (ACharacter::OnStartCrouch adjusts by the height change).
# Per-pawn state back to a freshly spawned AMordhauCharacter's (a respawn reuses this node): a new movement component
# (MordhauMovement defaults + the loadout's armor factors), standing capsule, zero velocity, LookUpValue (+0x520,
# a pawn field: `pitch`) 0. `yaw` is set by the caller (spawn facing).
func reset_pawn() -> void:
	move = MordhauMovement.load_default()
	if loadout != null:
		move.apply_loadout(loadout)
	velocity = Vector3.ZERO
	pitch = 0.0
	_crouch_held = false
	_sprint_held = false
	if shape != null:
		_set_half_height(CAPSULE_HALF_HEIGHT)
	if mesh_root != null:
		_place_mesh(CAPSULE_HALF_HEIGHT)

static func default_mesh(l: Loadout) -> Node3D:
	if l != null:
		return CharacterBuilder.build(l)
	var m := UeAnim.instance(UMA_MESH)
	if m != null: m.name = "Body"
	return m

func _place_mesh(half_height: float) -> void:
	var p := CharacterData.mesh()
	var loc := p.relative_location
	var z := loc.z + (CAPSULE_HALF_HEIGHT - half_height)
	# UE (x,y,z) -> glTF/Godot (x,z,y) (Gltf.SwapYZ); UE yaw about +Z, left-handed -> about +Y with the sign flipped
	mesh_root.position = Vector3(loc.x, z, loc.y) * CM
	mesh_root.rotation_degrees.y = -p.relative_rotation.y

func _set_half_height(hh: float) -> void:
	var c: CapsuleShape3D = shape.shape
	c.radius = CAPSULE_RADIUS * CM
	c.height = 2.0 * hh * CM   # Godot capsule height includes both caps = UE 2 * HalfHeight
	shape.position.y = 0.0

func control_basis() -> Basis:
	return Basis(Vector3.UP, deg_to_rad(yaw)) * Basis(Vector3.RIGHT, deg_to_rad(pitch))

func add_look(dx: float, dy: float) -> void:
	yaw -= dx
	pitch = clampf(pitch - dy, -look_down_limit, look_up_limit)

# The model half of step(): no node is read or moved, so it also runs outside the tree (the Rust port's golden traces,
# godot/tools/export_golden_character.gd, drive it). Button edges, crouch toggle, jump, then MordhauMovement.tick.
# Returns {delta: position delta in cm, facing: actor forward, crouch: true / false = Crouch() / UnCrouch() applied,
# null = no change}.
func step_model(dt: float, fwd: float, right: float, jump: bool, sprint: bool, crouch: bool) -> Dictionary:
	var b := Basis(Vector3.UP, deg_to_rad(yaw))
	var facing := -b.z
	var wish := (facing * fwd + b.x * right).limit_length(1.0)
	# button edges -> AMordhauCharacter::SprintPressed / SprintReleased / CrouchPressed / CrouchReleased (MordhauMovement)
	if sprint != _sprint_held:
		if sprint: move.sprint_pressed()
		else: move.sprint_released()
		_sprint_held = sprint
	move.move_forward_axis(fwd)
	if crouch != _crouch_held:
		if crouch: move.crouch_pressed()
		else: move.crouch_released()
		_crouch_held = crouch
	var c = move.update_crouch()		# AMordhauCharacter::LODTick crouch cooldown -> Crouch() / UnCrouch()
	if c != null and move.mode == MordhauMovement.Mode.WALKING:
		move.crouched = c
	else:
		c = null
	if jump: move.do_jump()
	return {"delta": move.tick(dt, wish, facing), "facing": facing, "crouch": c}

# Input -> model -> body. fwd/right: -1..1 axes ("Move Forward"/"Move Right").
func step(dt: float, fwd: float, right: float, jump: bool, sprint: bool, crouch: bool) -> void:
	rotation_degrees.y = yaw        # character yaw follows the control yaw
	var r := step_model(dt, fwd, right, jump, sprint, crouch)
	var facing: Vector3 = r.facing
	var delta_cm: Vector3 = r.delta
	var c = r.crouch
	if c != null:		# the capsule follows Crouch() / UnCrouch() (the model never reads the body)
		var hh := crouched_half_height if c else CAPSULE_HALF_HEIGHT
		var dz := (CAPSULE_HALF_HEIGHT - hh) * CM
		if shape != null: _set_half_height(hh)
		if is_inside_tree():
			global_position.y += -dz if c else dz   # keep the capsule bottom on the floor
		else:
			position.y += -dz if c else dz
		if mesh_root: _place_mesh(hh)
	velocity = delta_cm * CM / dt
	move_and_slide()
	move.set_on_floor(is_on_floor())
	if get_slide_collision_count() > 0 and move.mode == MordhauMovement.Mode.WALKING:
		var rv := get_real_velocity() / CM
		move.velocity.x = rv.x
		move.velocity.z = rv.z
	_animate(facing)

func _animate(facing: Vector3) -> void:
	if anim == null: return
	var v := Vector2(move.velocity.x, move.velocity.z)
	var speed := v.length()
	var dir := 0.0
	if speed > 1.0:
		# UAnimInstance::CalculateDirection: signed yaw of velocity relative to facing, + = right
		dir = rad_to_deg(Vector2(facing.x, facing.z).angle_to(v))
	var walk := move.cfg.max_walk_speed
	var run := walk * move.cfg.sprint_modifier
	var walk_run := clampf((speed - walk) / maxf(run - walk, 1.0), 0.0, 1.0)
	var upper_max := MordhauAnim.axis_max(MordhauAnim.BS_UPPER, 1)
	MordhauAnim.set_params(anim, dir, walk_run, lerpf(upper_max * 2.0 / 3.0, upper_max, walk_run),
		clampf(speed / walk, 0.0, 1.0), move.crouched, move.mode == MordhauMovement.Mode.FALLING)

func _physics_process(dt: float) -> void:
	if not InputMap.has_action("Jump"): return
	step(dt, MordhauInput.axis("Move Forward"), MordhauInput.axis("Move Right"),
		Input.is_action_just_pressed("Jump"), Input.is_action_pressed("Sprint"), Input.is_action_pressed("Crouch"))

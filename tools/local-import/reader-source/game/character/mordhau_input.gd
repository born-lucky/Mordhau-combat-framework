# mordhau_input.gd - Godot InputMap built from Mordhau's own Config/DefaultInput.ini (extract/config, `repak get`).
# +ActionMappings=(ActionName="Jump",...,Key=SpaceBar) -> action "Jump" on Space; +AxisMappings=(AxisName=
# "Move Forward",Scale=1.0,Key=W) -> action "Move Forward+" (Scale > 0) / "Move Forward-" (Scale < 0).
# Mouse axes come from the same file: AxisConfig MouseX/MouseY Sensitivity times BaseGame.ini InputYawScale /
# InputPitchScale (APlayerController::AddYawInput multiplies by them), giving degrees per mouse count.
class_name MordhauInput

# UE FKey names -> Godot keycodes, for the keyboard keys the character uses (EKeys names, InputCoreTypes).
const KEYS := {
	"W": KEY_W, "A": KEY_A, "S": KEY_S, "D": KEY_D, "P": KEY_P, "SpaceBar": KEY_SPACE,
	"LeftShift": KEY_SHIFT, "LeftControl": KEY_CTRL, "C": KEY_C, "V": KEY_V, "Q": KEY_Q, "E": KEY_E,
}
const ACTIONS := ["Jump", "Sprint", "Crouch", "Cycle Camera"]
const AXES := ["Move Forward", "Move Right"]

static func _add(action: String, key: String) -> void:
	if not KEYS.has(key): return
	if not InputMap.has_action(action): InputMap.add_action(action)
	var ev := InputEventKey.new()
	ev.physical_keycode = KEYS[key]
	InputMap.action_add_event(action, ev)

# Degrees of control rotation per mouse count
class MouseScale:
	var mouse_yaw_deg := 0.0
	var mouse_pitch_deg := 0.0

# Registers the actions / axes (UeConfig.input(): DefaultInput.ini) as a side effect; returns the mouse scales.
static func setup() -> MouseScale:
	var out := MouseScale.new()
	var ic := UeConfig.input()
	for m in ic.actions:
		if m.action in ACTIONS: _add(m.action, m.key)
	for x in ic.axes:
		if x.axis in AXES:
			_add(x.axis + ("+" if x.scale > 0.0 else "-"), x.key)
	var yaw := UeConfig.float_value("BaseGame.ini", "/Script/Engine.PlayerController", "InputYawScale")
	var pitch := UeConfig.float_value("BaseGame.ini", "/Script/Engine.PlayerController", "InputPitchScale")
	out.mouse_yaw_deg = ic.sensitivity_of("MouseX") * yaw
	# "Look Up" is mapped to MouseY with Scale=-1 and InputPitchScale is negative: mouse down (+y) pitches down.
	out.mouse_pitch_deg = ic.sensitivity_of("MouseY") * pitch * -1.0
	return out

# -1..1 axis value from the "+"/"-" action pair.
static func axis(name: String) -> float:
	var v := 0.0
	if InputMap.has_action(name + "+") and Input.is_action_pressed(name + "+"): v += 1.0
	if InputMap.has_action(name + "-") and Input.is_action_pressed(name + "-"): v -= 1.0
	return v

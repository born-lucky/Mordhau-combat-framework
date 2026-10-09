# bot_body.gd - what the bot code reads from a character (its own pawn or an enemy), in UE units: centimetres,
# Z up, yaw in degrees (DESIGN.md §4). The duel scene fills these from its nodes each frame (ue_from_godot below);
# the combat state is the fighter's MotionSystem (godot/game/combat).
#
# Field sources (AMordhauCharacter layout extract/native/types/AMordhauCharacter.h + engine parents):
#   location / yaw     RootComponent (+0x130) ComponentToWorld translation (+0x1d0) / GetForwardVector
#   velocity           APawn::GetVelocity (vtable +0x2f8)
#   capsule            CapsuleComponent (+0x290) CapsuleRadius +0x45c = 50, CapsuleHalfHeight +0x458 = 96: stored by
#                      AMordhauCharacter::AMordhauCharacter rva=0x1524d90 (0x42480000, 0x42c00000); scaled radius =
#                      CapsuleRadius * min(ComponentToWorld scale X, Y) (+0x1e0/+0x1e4), as the bot tasks compute it
#   max_walk_speed     CharacterMovement (+0x288) +0x18c. That offset is MaxWalkSpeed: UCharacterMovementComponent::
#                      GetMaxSpeed rva=0x2f79a50 returns +0x18c for MOVE_Walking (+0x190 when crouched). Value 308 =
#                      BP_MordhauCharacter CDO subobject CharMoveComp.MaxWalkSpeed (extract/json; set by MordhauBotController.make)
#   mesh_scale_x       Mesh (+0x280) RelativeScale3D.X (+0x134). Not in the CDO -> USceneComponent default 1.0
#                      (UNCONFIRMED: engine default, not read)
#   weapon_length      AMordhauWeapon::Length (+0x1bec) = |tracer span| / 15, computed at runtime from the weapon mesh
#                      sockets by AMordhauWeapon::RecalculateTracerPoints rva=0x163a940 (_DAT_1440701e0 = 1/15).
#                      Not in any CDO: the scene / hit-detection builder must set it (UNCONFIRMED until then)
#   trace_*            AMordhauWeapon CurrentTraceStart +0xd48, CurrentTraceEnd +0xd54, PreviousTraceEnd +0xd6c
#                      (hit detection fills these; only the perfect-parry branch of MeleeDefend reads them)
#   ignore_cache       AMordhauWeapon ActorIgnoreCache +0xe88: bodies this swing already hit
#   facing_bone_height GetSocketLocation(<FName at 0x145720590>).Z - Mesh Z, read by MeleeAttack for StartFacingBone.
#                      The FName is built at startup (not in .rdata), so the bone is UNCONFIRMED; the scene supplies it
class_name BotBody
extends RefCounted

var name := ""
var sys: MotionSystem				# +0x688 MotionSystemComponent
var team := 0						# AAdvancedCharacter +0x660 Team
var is_dead := false				# +0x504 bIsDead
var location := Vector3.ZERO
var yaw := 0.0
var velocity := Vector3.ZERO
var capsule_radius := 50.0
var capsule_half_height := 96.0
var capsule_scale_min := 1.0
var max_walk_speed := 308.0
var mesh_scale_x := 1.0
var weapon_length := 0.0
var trace_start := Vector3.ZERO
var trace_end := Vector3.ZERO
var prev_trace_end := Vector3.ZERO
var ignore_cache: Array = []
var facing_bone_height := 0.0
# outputs the bot writes (the scene applies them)
var wants_crouch := false			# AMordhauCharacter +0xde4 bWantsCrouch (StartCrouching / StopCrouching)
var wants_sprint := false			# AMordhauCharacter::StartSprinting rva=0x156dcc0 sets CharacterMovement +0xd19
# inventory for UBTTask_SwitchEquipment: [{weapon: WeaponData, is_fists: bool}]; slot 0 is held in the right hand
var inventory: Array = []
var right_hand := 0
var eye_height := 0.0				# APawn::GetActorEyesViewPoint height above the root (BaseEyeHeight; the scene sets it,
									# UNCONFIRMED until then)
var ai = null						# APawn Controller (+0x258) when it is an AMordhauAIController (MordhauBotController)

func _init(nm := "", s: MotionSystem = null) -> void:
	name = nm
	sys = s
	if s != null:
		inventory = [{"weapon": s.weapon, "is_fists": false}]

# USceneComponent::GetForwardVector of an upright root: (cos yaw, sin yaw, 0)
func forward() -> Vector3:
	var r := deg_to_rad(yaw)
	return Vector3(cos(r), sin(r), 0.0)

# UCapsuleComponent::GetScaledCapsuleRadius as inlined in the tasks: min(scale X, scale Y) * CapsuleRadius
func scaled_radius() -> float:
	return capsule_scale_min * capsule_radius

# the speed term the tasks use: max(|GetVelocity().XY|, CharacterMovement +0x18c MaxWalkSpeed)
func threat_speed() -> float:
	var s := Vector2(velocity.x, velocity.y).length()
	return maxf(max_walk_speed, s)

func stamina_byte() -> int:
	return sys.stamina_byte()

# AMordhauCharacter Health byte (the replicated 0..100 value the Blueprints compare)
func health_byte() -> int:
	return sys.health_byte()

# Godot metres, Y up -> UE centimetres, Z up: the inverse of CUE4Parse's glTF conversion (x, z, y) * 0.01 (DESIGN.md §4)
static func ue_from_godot(p: Vector3) -> Vector3:
	return Vector3(p.x, p.z, p.y) * 100.0

static func godot_from_ue(p: Vector3) -> Vector3:
	return Vector3(p.x, p.z, p.y) * 0.01

# UE yaw (degrees) of a facing direction given in Godot space (e.g. -global_basis.z of the character)
static func yaw_from_godot_dir(d: Vector3) -> float:
	var u := Vector3(d.x, d.z, d.y)
	return rad_to_deg(atan2(u.y, u.x))

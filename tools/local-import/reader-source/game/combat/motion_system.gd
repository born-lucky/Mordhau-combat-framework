# motion_system.gd - one fighter: UMotionSystemComponent (current motion, NetMotion, motion switching) plus the
# parts of AMordhauCharacter, UStaminaStatComponent and UHealthStatComponent the combat motions read and write.
# Sources: extract/native/decomp/UMotionSystemComponent.cpp, AMordhauCharacter.cpp, UStaminaStatComponent.cpp,
# UStatComponent.cpp. One machine, authority role (ROLE_Authority = 3): no prediction, no ping, ExpectedDelay 0.
class_name MotionSystem
extends RefCounted

# FNetMotion (types/FNetMotion.h). One per fighter (UMotionSystemComponent +0xc4 NetMotion).
class NetMotion:
	var id := 0					# Id (+0x0): UMotionSystemComponent::AssignNetMotion rva=0x14b34c0 sets NetMotion.Id + 1
	var motion_type := 0		# CombatEnums.NetType
	var param0 := 0
	var param1 := 0
	var param2 := 0
	var dynamic_param := 0
	func _init(t := 0, p0 := 0, p1 := 0, p2 := 0, dyn := 0) -> void:
		motion_type = t; param0 = p0 & 0xff; param1 = p1 & 0xff; param2 = p2 & 0xff; dynamic_param = dyn & 0xff

var name := ""
var _world_ref: WeakRef		# -> CombatState. Weak: the world owns its fighters (CombatState.fighters), never the reverse
var world: CombatState:		# CombatState (clock, trace, hit processing); null once the world is freed
	get: return _world_ref.get_ref() if _world_ref != null else null
var id := 0					# this pawn's identity for cross-fighter lists (hit lists, ignore caches): the object's instance
							# id, stored as an int so no fighter holds another one (no reference cycles between fighters)
var weapon: WeaponData				# RightHandEquipment's combat numbers (this fighter's copy; SwitchMode swaps it)
var weapon_equip: EquipmentDef		# RightHandEquipment's equipment fields (profile, Length, ...)
var motion_bps := {}		# EAttackMove -> motion Blueprint path (animation profile Attacks map)
var parry_bp := ""
var left_hand_path := ""			# AMordhauCharacter +0x1200 LeftHandEquipment (Blueprint path, "" = none; e.g. a buckler
								# carried with a one-hander)
var left_hand: EquipmentDef = null	# its equipment fields (null = none)
var left_weapon: WeaponData = null	# its combat numbers when it IsA AMordhauWeapon (shields), else null
var character: CharacterData.Character	# AMordhauCharacter class defaults (shared record: read-only)
var kick_weapon: WeaponData = null	# AMordhauCharacter +0x1210 KickWeapon (CombatData.kick_weapon_path: BP_KickWeapon)
var has_last_chance := false		# AAdvancedCharacter +0x9f0 bHasLastChance, this pawn's copy (cleared when used)
var stamina_stat: CharacterData.Stat	# UStaminaStatComponent ctor defaults
var health_stat: CharacterData.Stat		# UHealthStatComponent ctor defaults
var tracer: WeaponTracer

var motion: CombatMotion			# +0xe8 Motion
var last_attack_motion = null		# +0x108
var last_feinted_motion = null		# +0x100
var last_parry_motion = null		# +0xf0
var net := NetMotion.new()			# +0xc4 NetMotion
var next_kick_time := 0.0			# +0xb0
var next_attack_time := 0.0			# +0xb4
var next_available_stun_time := 0.0	# +0xb8 NextAvailableStunTime (written by UStunMotion::OnTick; no native reader found)
var stamina := 100					# stamina UStatComponent +0xb0 StatValue
var next_stamina_regen := 0.0		# stamina UStatComponent +0xb4 NextRegenerationTick
var stamina_regenerable := true		# this pawn's stamina UStatComponent bIsRegenerable (class default stamina_stat; cleared on death)
var health := 100					# health UStatComponent +0xb0 StatValue: int32 (types/UStatComponent.h)
var easy_parry_until_time := 0.0	# AMordhauCharacter +0xe94 EasyParryUntilTime
var armor_tier_override := -1		# AAdvancedCharacter +0x7f4 DamageArmorTierOverride (ctor: 0xffffffff)
var wearable_coverage := {}			# AMordhauCharacter +0xb18 WearableProtectionCoverageMap: bone -> ArmorClass
var airborne := false				# AAdvancedCharacter +0x839 ReplicatedCharacterFlags bit 0
var airborne_time := 0.0			# AAdvancedCharacter +0x86c AirborneTime (UKickMotion::ModifyAttackInfo jump kick), fed by the game
var holding_block := false			# AMordhauCharacter +0xb99 bIsHoldingBlock (IsHoldingBlock rva=0x154aa50 also ORs
									# bIsHoldingFeintOrBlock +0xb10, which no input here sets)
var wants_block := false			# AMordhauCharacter +0x1029 bWantsBlock (BlockPressed retry, see tick)
var look_up_value := 0.0			# AAdvancedCharacter +0x520 LookUpValue, degrees: AAdvancedCharacter::LookUp
									# rva=0x1489930 clamps it to [-LookDownLimit, LookUpLimit]; the game feeds pitch in
var b_is_left_arm_disabled := false	# AMordhauCharacter +0xd40 bIsLeftArmDisabled (dismemberment; not ported: stays false)
var b_is_right_arm_disabled := false	# AMordhauCharacter +0xd41 bIsRightArmDisabled
var alternate_mode := false			# weapon mode after SwitchMode (switch_mode below)
var bone_xf := {}					# bone name -> world Transform3D (Godot), fed by the game each tick
var actor_xf = null					# actor (capsule) world Transform3D in the same convention as bone_xf (maps
									# UePhysics.swap(UE local) x 0.01 to Godot world), fed by the game; null = unknown
									# (headless): the parry's BlockCollider sweep is skipped
var dead := false
# Networking (game/net, phase P10). One machine (no game/net) = authority, no replication, every fighter locally
# controlled: role 3 and net_rep null, which keeps every path below as it was.
var role := 3						# AActor Role on this machine, ENetRole order of the exe's name table (ROLE_None
									# 0x144b1f260, ROLE_SimulatedProxy 0x144b1f270, ROLE_AutonomousProxy 0x144b1f288,
									# ROLE_Authority 0x144b1f2a0): 1 simulated, 2 autonomous (owning client), 3 authority
var remote_controlled := false		# authority copy of a pawn whose controller is on a remote client (not locally controlled)
var net_rep = null					# NetMotionRep (game/net): the replication half of UMotionSystemComponent; null = no net
var team := 255						# AMordhauPlayerState Team, set by the game mode; 255 = none (IsFriendly_Implementation rva=0x159bc10 tests 0xff)
var creation_time := 0.0			# AActor +0x9c CreationTime (CombatState.add_fighter)
# Optional pose provider for the AutoBlend search (attack_motion.gd OnBegin): the animation layer's clip choice and
# its poses, so combat's windup offset matches the animation shown
class AnimPoses:
	var clip: Callable		# (AttackMotion) -> sequence path, "" = none or a montage
	var now: Callable		# () -> AttackAutoBlend.Pose shown now
	var at: Callable		# (clip, t) -> AttackAutoBlend.Pose of the clip at t
var anim_poses: AnimPoses = null

func _init(w: CombatState, nm: String, weapon_path: String, left_path := "") -> void:
	_world_ref = weakref(w)
	id = get_instance_id()
	name = nm
	var s := CombatData.weapon_setup(weapon_path)
	weapon = s.weapon
	weapon_equip = s.equip
	motion_bps = s.motions
	left_hand_path = left_path
	if left_path != "":
		left_hand = EquipmentDef.load_def(left_path)
		if left_hand != null and left_hand.is_weapon:
			left_weapon = UeWeapon.load_weapon(left_path)
	parry_bp = parry_motion_bp()
	character = CharacterData.character()
	var kp := CombatData.kick_weapon_path()
	if kp != "":
		kick_weapon = UeWeapon.load_weapon(kp)
	has_last_chance = character.b_has_last_chance
	stamina_stat = CharacterData.stat("UStaminaStatComponent")
	health_stat = CharacterData.stat("UHealthStatComponent")
	armor_tier_override = character.damage_armor_tier_override
	# UStaminaStatComponent::InitializeComponent rva=0x14fdd40 / UStatComponent::InitializeComponent rva=0x14fde40:
	# StatValue = InitialStatValue
	stamina = stamina_stat.initial_value
	health = health_stat.initial_value
	tracer = WeaponTracer.new(weapon)
	change_to_idle()

func now() -> float:
	return world.now

# The drop of UDisarmedMotion::OnBegin_Implementation rva=0x165ee70 / UStunMotion::OnBegin_Implementation rva=0x16628e0
# (bWillDisarm), authority only (Role 3; every fighter here is authority unless game/net says otherwise):
# ToDrop = the ComingFromMotion's Weapon (+0x10c8) when it IsA UAttackMotion, else LeftHandEquipment when it IsA
# AMordhauWeapon, else RightHandEquipment when it IsA AMordhauWeapon; AMordhauCharacter::DropEquipment(ToDrop); then
# with no RightHandEquipment and no CurrentVehicle, AMordhauCharacter::SwitchToFists rva=0x156e920 (equip the last
# Equipment slot through UEquipmentSystemComponent::SwitchEquipment). The combat port has no inventory: the drop clears
# the hand here and the `drop_equipment` / `switch_to_fists` events ask the game layer (which owns the loadout) to
# act; it calls equip_right with the fists. UNCONFIRMED: the SwitchEquipment motion's timing (not ported).
func drop_for_disarm(from) -> void:
	if not world.authority:				# CombatState.authority: false on a client's copy
		return
	var hand := ""
	if from is AttackMotion and from.weapon != null and from.weapon != kick_weapon:
		hand = "right" if from.weapon == weapon else "left"
	elif _left_is_weapon():
		hand = "left"
	elif weapon != null:
		hand = "right"
	if hand == "":
		return
	var path := left_hand_path if hand == "left" else (weapon_equip.path if weapon_equip != null else "")
	if hand == "left":
		left_hand_path = ""
		left_hand = null
		left_weapon = null
	else:
		weapon = null
		weapon_equip = null
	parry_bp = parry_motion_bp()
	trace_event("dropped %s %s" % [hand, path.get_file()])
	world.emit_event({"kind": "drop_equipment", "who": name, "hand": hand, "path": path})
	if weapon == null:
		world.emit_event({"kind": "switch_to_fists", "who": name})

# The game layer's answer to `switch_to_fists` (or any re-equip): RightHandEquipment = that weapon Blueprint
func equip_right(weapon_path: String) -> void:
	var s := CombatData.weapon_setup(weapon_path)
	weapon = s.weapon
	weapon_equip = s.equip
	motion_bps = s.motions
	alternate_mode = false
	parry_bp = parry_motion_bp()
	tracer = WeaponTracer.new(weapon)

# UAttackMotion::FindWeapon_Implementation rva=0x161d9f0: the RightHandEquipment if it IsA AMordhauWeapon;
# UKickMotion::FindWeapon_Implementation rva=0x165b4b0: the character's KickWeapon (+0x1210). The attack reads its
# AttackInfo (GetBaseAttackInfo), StabReleaseModifier and AttackMask from that weapon.
func find_weapon(attack_native_class: String) -> WeaponData:
	if CombatData.is_class_of(attack_native_class, "UKickMotion"):
		return kick_weapon
	return weapon

# UMotionSystemComponent::GetAttackMotionClass rva=0x14bc010: profile Attacks[move], else UAttackMotion.
# (exe wins, state/proofs/shield_motions.md) the LEFT-hand equipment's profile, when it is a weapon (a shield:
# BP_ShieldAnimationProfile), is looked up last and its entry replaces the right hand's (LAB_1414bc1f5, decomp
# UMotionSystemComponent.cpp 520-575)
func attack_motion_defaults(m: int) -> MotionDefs.Attack:
	var p: String = motion_bps.get(m, "")
	if _left_is_weapon() and left_hand != null and left_hand.weapon_animation_profile != "":
		var lp: String = CombatData.profile_motions(left_hand.weapon_animation_profile).motions.get(m, "")
		if lp != "":
			p = lp
	if p == "":
		return CombatData.native_motion_def("UAttackMotion")
	return CombatData.motion_def(p)

# UMotionSystemComponent::CanPerformAttack rva=0x14b4df0: the owner's RightHandEquipment (+0x11f8), else
# LeftHandEquipment (+0x1200), ->CanPerformAttack(Character, Move) through ProcessEvent (FName global
# NAME_AMordhauEquipment_CanPerformAttack, labels_data.tsv 0x145728328). No Blueprint in extract/json overrides
# CanPerformAttack (no "CanPerformAttack" function export in any package), so the native body runs:
# AMordhauEquipment::CanPerformAttack_Implementation rva=0x15328a0 (AFistsWeapon's own override is not used here):
#   !bCanAttack (+0xcb9) -> false; Couch with Stamina byte (+0xe67) == 0 -> false; on foot: !bCanAttackOnFoot -> false;
#   Kick with a disabled leg, or with MovementMode (movement component +0x168) == 3 while !bCanJumpKick -> false;
#   on a ladder only right-side moves. No vehicles / ladders / disabled legs exist in this simulation.
# The movement component (+0x288) and its +0x168 byte have no PDB header here (engine ACharacter /
# UCharacterMovementComponent); "MovementMode 3 = airborne" is read as `airborne` (UNCONFIRMED naming).
func can_perform_attack(m: int) -> bool:
	if weapon == null or not weapon.b_can_attack:
		return false
	if m == CombatEnums.Move.COUCH and stamina_byte() == 0:
		return false
	if not weapon.b_can_attack_on_foot:
		return false
	if m == CombatEnums.Move.KICK and airborne and not character.b_can_jump_kick:
		return false
	return true

# UMotionSystemComponent::ChangeMotion / ChangeMotion_Internal: old motion leaves, new one begins at `now`.
# Order, from UMotionSystemComponent::ChangeMotion_Internal rva=0x14b4f20 (decomp UMotionSystemComponent.cpp
# 1719-1745): Motion (+0xe8) = new and AMordhauCharacter::InternalSetMotion first, then new ComingFromMotion = old,
# then old UMordhauMotion::Interrupt rva=0x165d5e0 (LeaveTime = now, OnLeave(true)) - so the old motion's OnLeave
# already sees the new one as current -, then (character bIsUnflinchable: new bIsFlinchable = false; no native code
# writes that flag and no fighter here sets it, not modelled), then UMordhauMotion::Initialize rva=0x165d4c0:
# StartTime = now (+ the skipped delta time when bAppendAppDeltaTime: game/net), EndTime = StartTime, OnBegin, and
# OnTick with DeltaTime 0 (UMordhauMotion::Initialize disasm 0x14165d5c5 call UMordhauMotion::OnBegin, then
# 0x14165d5ca `xorps xmm1, xmm1` and tail jump 0x14165d5da to UMordhauMotion::OnTick rva=0x16d6990). That first
# OnTick is not UMordhauMotion::Tick: no EndTime test follows it.
func _change(m: CombatMotion) -> void:
	var old := motion
	motion = m
	m.coming_from = old
	if old != null:
		old.leave_time = now()
		old.on_leave(true)
	m.start_time = now()
	m.end_time = m.start_time
	if net_rep != null:		# game/net: bInitiatedLocally / bWasConfirmedByAuthority / ExpectedDelay, set before Initialize
		net_rep.init_motion(m)
	m.on_begin()
	if m is AttackMotion:
		last_attack_motion = m		# UAttackMotion::OnBegin_Implementation stores it near its end, before OnTick
	m.on_tick(0.0)				# on `m` even when its OnBegin already changed the motion (Blocked's replayed attack)
	_bound_history()
	world.trace_motion(self)

# Memory bound of the motion history (port design, no exe counterpart: UE's garbage collector frees unreachable
# motions; here the links themselves are cut). Roots = the motions this component points at (Motion +0xe8,
# LastParryMotion +0xf0, LastFeintedMotion +0x100, LastAttackMotion +0x108). Every rule reads at most one link from a
# root: the current motion's ComingFromMotion (UParryMotion::OnBegin, UAttackMotion::ProcessBlock,
# ProcessHitForDamage's riposte-trade test), an attack's PreviousLastAttackMotion (OnBegin, CheckChamber, the morph's
# chamber window) and a Blocked motion's attack; the animation layer reads one more hop from the motion that just left
# (motion_anim.gd _on_feinted: old.previous_last_attack). So the links of every motion two hops from a root are
# never read again, and cutting them keeps each fighter at a bounded number of live motions however long it fights.
func _bound_history() -> void:
	var roots := [motion, last_attack_motion, last_parry_motion, last_feinted_motion]
	for r in roots:
		if r == null:
			continue
		for hop1 in _links(r):
			for hop2 in _links(hop1):
				if not roots.has(hop2):
					_cut_links(hop2)

static func _links(m) -> Array:
	var out := []
	if m.coming_from != null:
		out.append(m.coming_from)
	if m is AttackMotion and m.previous_last_attack != null:
		out.append(m.previous_last_attack)
	if m is BlockedMotion and m.from_attack != null:
		out.append(m.from_attack)
	return out

static func _cut_links(m) -> void:
	m.coming_from = null
	if m is AttackMotion:
		m.previous_last_attack = null
	elif m is BlockedMotion:
		m.from_attack = null

func change_to_idle() -> void:
	_change(CombatMotion.Idle.new(self, CombatData.native_motion_def("UIdleMotion")))

# from UMotionSystemComponent::AssignNetAttackMotion rva=0x14b3210
func assign_net_attack_motion(type: int, m: int, angle: float) -> void:
	if not can_perform_attack(m):
		return
	var t := now()
	var comp := 0.0			# ping compensation (AutonomousProxy only) is 0 on the authority
	if net_rep != null:		# game/net: GetPing x TimeDilation against the motion's GetAttackCompensationStartTime
		comp = net_rep.attack_ping_compensation(m)
	var kick := 0.0
	if m == CombatEnums.Move.KICK:
		var k3 := CombatConstants.kick_lag_window
		kick = k3 - clampf(next_kick_time - t, 0.0, k3)
	# single precision as in `UMotionSystemComponent::AssignNetAttackMotion` at 0x1414b33e8 (addss), 0x1414b341c (mulss),
	# 0x1414b3460 (cvttss2si): 0.08 x 500 = 40, not 39
	var lag_f := PackedFloat32Array([clampf(comp, 0.0, CombatConstants.max_ping_compensation) + kick])
	lag_f[0] = lag_f[0] * CombatConstants.net_lag_units_per_s
	var lag := int(lag_f[0]) & 0xff
	var a := clampf(angle, CombatConstants.attack_angle_min, CombatConstants.attack_angle_max) * CombatConstants.attack_angle_to_unit
	_assign(NetMotion.new(CombatEnums.NetType.ATTACK, CombatEnums.pack(type, m), pack_float(a, -1.0, 1.0), lag))

# UMordhauUtilityLibrary::PackFloat rva=0x14cfde0: int(clamp01((v - lo) / (hi - lo)) * 255), degenerate range ->
# 255 if v >= hi else 0
static func pack_float(v: float, lo: float, hi: float) -> int:
	var span := hi - lo
	var f := 0.0
	if absf(span) > CombatConstants.small_number:
		f = (v - lo) / span
	elif hi <= v:
		return int(CombatConstants.byte_max)
	return int(clampf(f, 0.0, 1.0) * CombatConstants.byte_max)

# FNetMotion::Parry(EBlockType) as built by UMordhauMotion::ProcessBlock_Implementation: MotionType 2, Param0 = type
func assign_net_parry(bt: int) -> void:
	_assign(NetMotion.new(CombatEnums.NetType.PARRY, bt))

# FNetMotion::Feinted(EFeintType, EAttackMove) as built by UAttackMotion::ProcessFeint_Implementation: MotionType 5
func assign_net_feint(ft: int, m: int) -> void:
	_assign(NetMotion.new(CombatEnums.NetType.FEINTED, ft, m))

# FNetMotion::Blocked rva=0x1615f50: MotionType 6, Param0 = reason, Param1 = FBlockResult bools,
# Param2 = time x 40 (CombatConstants.blocked_time_to_byte) clamped to a byte (UAttackMotion::PutUsInBlockedMotionFromParry rva=0x163a7d0)
func assign_net_blocked(reason: int, flags: int, time_s: float) -> void:
	var b := clampf(time_s * CombatConstants.blocked_time_to_byte, 0.0, CombatConstants.byte_max)
	_assign(NetMotion.new(CombatEnums.NetType.BLOCKED, reason, flags, int(b)))

# FNetMotion::Flinched rva=0x14bb820: Param0 = PackFloat(angle / 180), Param1 = flag,
# Param2 = PackFloat(FlinchDurationModifier, [0.5, 2]), DynamicParam = PackFloat(FlinchSpeedModifier, [0, 1])
# FNetMotion::Stunned rva=0x14d86a0: MotionType 4, Param0 = PackFloat(direction x 1/180, [-1, 1]), Param1 = bone,
# Param2 = bShouldDisarm
func assign_net_stunned(direction: float, bone: int, should_disarm: bool) -> void:
	_assign(NetMotion.new(CombatEnums.NetType.STUNNED, pack_float(direction * CombatConstants.flinch_angle_to_unit, -1.0, 1.0),
		bone, int(should_disarm)))

# FNetMotion::Disarmed rva=0x14ba4a0: MotionType 7, Param0 = PackFloat(direction x 1/180, [-1, 1])
func assign_net_disarmed(direction: float) -> void:
	_assign(NetMotion.new(CombatEnums.NetType.DISARMED, pack_float(direction * CombatConstants.flinch_angle_to_unit, -1.0, 1.0)))

func assign_net_flinched(angle: float, flag: bool, duration_mod: float, speed_mod: float) -> void:
	_assign(NetMotion.new(CombatEnums.NetType.FLINCHED, pack_float(angle * CombatConstants.flinch_angle_to_unit, -1.0, 1.0),
		int(flag), pack_float(duration_mod, 0.5, 2.0), pack_float(speed_mod, 0.0, 1.0)))

# UMotionSystemComponent::AssignNetMotion rva=0x14b34c0 -> a new motion object of the class for MotionType.
# NewNetMotion.Id = NetMotion.Id + 1 on every machine. With game/net attached the role-dependent rest of AssignNetMotion
# (ServerAssignNetMotion / ClientSetNetMotion / HandleNetMotionUpdate) runs in NetMotionRep; alone, the authority's
# HandleNetMotionUpdate reduces to "NetMotion = new, create its motion".
func _assign(nm: NetMotion) -> void:
	nm.id = (net.id + 1) & 0xff
	if net_rep != null:
		net_rep.assign_net_motion(nm)
		return
	net = nm
	begin_net_motion()

# The motion-creating tail of UMotionSystemComponent::HandleNetMotionUpdate rva=0x14c01b0 for the current `net`:
# NewObject of the class for MotionType (Attack 1 -> GetAttackMotionClass(Param0 & 0xf), Parry 2 ->
# GetParryMotionClass), ChangeMotion_Internal, then DynamicParamChanged(0, MotionDynamicParam) when it is not 0.
# MotionType -> class: AMordhauCharacter::Motions (+0x10d0) indexed by MotionType (HandleNetMotionUpdate disasm
# 0x1414c0459..0x1414c04a9); the classes used below are that table's BP_MordhauCharacter CDO entries 2/3/5/6
# (test_combat test_motion_class_table_from_character checks them against the CDO).
func begin_net_motion() -> void:
	var nm := net
	match nm.motion_type:
		CombatEnums.NetType.ATTACK:
			_change(AttackMotion.new(self, attack_motion_defaults(CombatEnums.lo(nm.param0))))
		CombatEnums.NetType.PARRY:
			_change(ParryMotion.new(self, CombatData.motion_def(parry_bp) if parry_bp != "" \
				else CombatData.native_motion_def("UParryMotion")))
		CombatEnums.NetType.FEINTED:
			_change(FeintedMotion.new(self, CombatData.motion_def(CombatData.FEINTED_MOTION)))
		CombatEnums.NetType.BLOCKED:
			_change(BlockedMotion.new(self, CombatData.motion_def(CombatData.BLOCKED_MOTION)))
		CombatEnums.NetType.FLINCHED:
			_change(FlinchMotion.new(self, CombatData.motion_def(CombatData.FLINCH_MOTION)))
		CombatEnums.NetType.STUNNED:
			_change(StunMotion.new(self, CombatData.motion_def(CombatData.STUN_MOTION)))
		CombatEnums.NetType.DISARMED:
			_change(DisarmedMotion.new(self, CombatData.motion_def(CombatData.DISARMED_MOTION)))
		_:
			return
	if net.dynamic_param != 0 and motion != null:
		motion.on_dynamic_param_changed(0, net.dynamic_param)

# from UMotionSystemComponent::AssignNetMotionDynamicParam rva=0x14b35f0 (authority: set, notify the motion)
func assign_net_motion_dynamic_param(v: int) -> void:
	if net_rep != null:
		net_rep.assign_net_motion_dynamic_param(v)		# Role 3 only, then ClientSetNetMotion + OnRep (game/net)
		return
	var old: int = net.dynamic_param
	net.dynamic_param = v & 0xff
	if old != net.dynamic_param and motion != null:
		motion.on_dynamic_param_changed(old, net.dynamic_param)
	world.trace_event(name + " dyn " + str(net.dynamic_param))

# AMordhauCharacter::ServerDropParry_Implementation rva=0x1567ad0 (disassembled: 0x141567ad0..0x141567b3a):
#   return if bIsDead (+0x504), if MotionSystem NetMotion.Id (+0xc4) != MotionID, or if the current Motion (+0xe8) is
#   not a UParryMotion (UParryMotion::StaticClass 0x141710970); else tail-jump to UParryMotion::StopHolding.
# The caller passes its own current NetMotion Id (UParryMotion::OnTick_Implementation / ProcessFeint_Implementation).
# Called on a client (Role != 3) it is the reliable server RPC (game/net NetMotionRep.send_server_drop_parry); on the
# authority it runs here directly.
func server_drop_parry() -> void:
	if net_rep != null and role != 3:
		net_rep.send_server_drop_parry(net.id)
		return
	server_drop_parry_implementation(net.id)

func server_drop_parry_implementation(motion_id: int) -> void:
	if dead or net.id != motion_id or not (motion is ParryMotion):
		return
	motion.stop_holding()

# The gate in front of UParryMotion::OnTick_Implementation's ServerDropParry call (rva=0x1668980, decomp
# UParryMotion.cpp 375-390): IsLocallyControlled (vcall +0x9c8), or Role == 3 with no Controller (vcall +0x9c0).
# An owning client (role 2) and an authority pawn not driven from a remote client pass; the server's copy of a remote
# player's pawn (it has a PlayerController) and simulated proxies do not.
func drops_parry_locally() -> bool:
	return role == 2 or (role == 3 and not remote_controlled)

# APawn::IsLocallyControlled (vcall +0x9c8) in the role model above: the owning client's pawn (role 2) and, on the
# authority, every pawn not driven from a remote client (bots, the listen host). Without game/net: every fighter.
func is_locally_controlled() -> bool:
	return role == 2 or (role == 3 and not remote_controlled)

# from UStaminaStatComponent::OffsetStamina rva=0x1507c20 (costs scaled by AMordhauCharacter StaminaCostModifier
# +0xe44) and UStaminaStatComponent::SetStatValue_Internal rva=0x1515c80 (clamp: < Min -> Min, >= Max -> Max)
# Only the authority applies it: OffsetStamina tests the owner's Role == 3 before SetStatValue_Internal(v, true), whose
# bReplicate writes the ReplicatedStamina byte (net_rep.write_replicated_stamina).
func offset_stamina(v: int) -> void:
	var mod := character.stamina_cost_modifier
	if v < 0 and mod != 1.0:
		v = int(float(v) * mod)
	if role != 3:
		return
	var old := stamina
	_set_stamina(stamina + v)
	if net_rep != null and stamina != old:
		net_rep.write_replicated_stamina()
	world.trace_event("%s stamina %+d -> %d" % [name, v, stamina])

func _set_stamina(nv: int) -> void:
	var mn := stamina_stat.min_value
	var mx := stamina_stat.max_value
	stamina = mn if nv < mn else (nv if nv < mx else mx)

# AMordhauCharacter +0xe67 Stamina byte = RoundToInt(100 * (StatValue - Min) / (Max - Min)) (SetStatValue_Internal)
func stamina_byte() -> int:
	var mn := float(stamina_stat.min_value)
	var mx := float(stamina_stat.max_value)
	var f := clampf((float(stamina) - mn) / (mx - mn), 0.0, 1.0) if mx != mn else 1.0
	return int(round(f * 100.0))

# AMordhauCharacter::StopStaminaRegen rva=0x156df00 / UStatComponent::StopRegeneration rva=0x1516820:
# NextRegenerationTick = max(now + RegenerationStoppedDelay + extra, NextRegenerationTick); the stamina component's
# RegenerationStoppedDelay is the character's StaminaRegenDelay (UStaminaStatComponent::TickStat rva=0x1519900 copies
# +0xe6c into +0xc8 every tick).
func stop_stamina_regen(extra := 0.0) -> void:
	next_stamina_regen = maxf(now() + character.stamina_regen_delay + extra, next_stamina_regen)

# from UStatComponent::TickStat rva=0x1519ab0 with the stamina parameters UStaminaStatComponent::TickStat copies in:
# RegenerationPerTick = StaminaRegenPerTick (+0xe69), RegenerationTickRate = StaminaRegenTickRate (+0xe78)
func _tick_stamina_regen() -> void:
	if not stamina_stat.b_is_regenerable or not stamina_regenerable:
		return
	var per := character.stamina_regen_per_tick
	var rate := character.stamina_regen_tick_rate
	var mx := stamina_stat.max_value
	var mn := stamina_stat.min_value
	if not ((per >= 1 and stamina < mx) or (per < 0 and mn < stamina)):
		return
	var t := now()
	if t <= next_stamina_regen:
		return
	while true:
		var nv := stamina + per
		next_stamina_regen += rate
		if stamina != nv:
			_set_stamina(nv)
		if (per >= 1 and mx <= stamina) or (per < 1 and (per >= 0 or stamina <= mn)):
			break
		if t <= next_stamina_regen:
			return
	next_stamina_regen = t + rate

# EMordhauDamageType values the damage path compares with: Melee = 1, Ranged = 2 (UDamageableComponent::
# OnPostTakeDamage disassembly 0x141490c26..0x141490c35: `cmp cl, 1` / `cmp cl, 2` on DamageInfo.Type +0x8c).
const DAMAGE_MELEE := 1
const DAMAGE_RANGED := 2
const DAMAGE_FALL := 3		# EMordhauDamageType::Fall (types/EMordhauDamageType.h; UAdvancedCharacterMovement::CheckFallDamage)

# Melee damage on this character, the way ProcessHitForDamage's TakeDamage (vtable +0x590) applies it. Chain, all
# read from extract/native/decomp:
#   AMordhauCharacter::TakeDamage rva=0x156e980 -> AAdvancedCharacter::TakeDamage rva=0x14a3930 ->
#   1. AAdvancedCharacter::ModifyDamage (Blueprint event; only Horde/dummy Blueprints override it, so the native
#      ModifyDamage_Implementation runs: `movaps xmm0, xmm1; ret` at 0x141489ee0 = the damage unchanged)
#   2. the character's Received* fields are copied into its UDamageableComponent (disassembly 0x1414a3bfa..
#      0x1414a3c4f) and UDamageableComponent::ModifyDamage rva=0x1489be0 runs (_modify_damage)
#   3. UDamageableComponent::OnPostTakeDamage rva=0x1490ad0 stores the health (_post_take_damage)
# Skipped as the code is written for this world: the AMordhauGameMode / AMordhauGameState branches (spawn protection
# x0.02, team damage, damage-history credit) need a game mode, which CombatState does not have.
# Returns the damage as it was applied (TakeDamage's return value).
# source: the instigating fighter (the attacker), null for none.
# AMordhauCharacter::TakeDamage rva=0x156e980 (disasm 0x14156e980..0x14156ef21), before AAdvancedCharacter::TakeDamage,
# with an AMordhauGameMode (world.mode_rules):
#   now - CreationTime < GameMode SpawnProtectionDuration (+0x418) and Damage < spawn_protection_max_damage (5000) -> 0
#   FMordhauDamageEvent (type id 100, FMordhauDamageEvent::GetTypeID 0x141485aa0): IsFriendly(source, self) and
#     now - CreationTime < friendly_spawn_window (5 s) -> Damage x spawn_damage_scale (0.02)
# type: EMordhauDamageType (default Melee); game/character passes DAMAGE_FALL for CheckFallDamage's TakeDamage.
func take_damage(amount: float, source = null, type := DAMAGE_MELEE) -> float:
	if dead:
		return 0.0			# AAdvancedCharacter::TakeDamage: vcall +0x698 (ShouldTakeDamage) false -> 0
	var r = world.mode_rules
	var age: float = world.now - creation_time
	if r != null:
		if age < r.spawn_protection_duration and amount < CombatConstants.spawn_protection_max_damage:
			amount = 0.0
		if world.is_friendly(source, self) and age < CombatConstants.friendly_spawn_window:
			amount *= CombatConstants.spawn_damage_scale
	var applied := _modify_damage(amount, type, source)
	_post_take_damage(applied, type)
	return applied

# from UDamageableComponent::ModifyDamage rva=0x1489be0 (Melee type; no game mode / game state):
#   d = Damage x DamageModifier (= AAdvancedCharacter ReceivedDamageModifier +0x9f8)
#   d > 0: d = max(d - DamageAbsorption (+0xa0c), 0); then if d > 0: d = max(d, CombatConstants.min_damage = 1.0),
#   and MaxDamage (ReceivedDamageMax +0xa10) > 0 caps it; finally d = min(d, float(current health)).
# With a game mode (disasm 0x141489c22..0x141489e14): when the component's SpawnProtectionDuration (+0xe4, ctor 0.2)
# > 0: now - owner CreationTime < it -> d x spawn_damage_scale (0.02) (also when component flag +0x8b bit 0 is clear:
# UNCONFIRMED, taken as set for a live character; the >= 999 player-controller exemption at +0x6d0 is not modelled);
# then d x AMordhauGameMode::GetDamageFactor_Implementation rva=0x15959c0 (bDisableDamage -> 0; else DamageFactor,
# x TeamDamageFactor when IsFriendly(instigator, owner)). After the type modifiers and DamageModifier: IsFriendly ->
# d x TeamDamageModifier (+0xc8 = character ReceivedTeamDamageModifier, copied by OnTakeDamage rva=0x14922d0).
func _modify_damage(amount: float, type: int, source = null) -> float:
	var d := amount
	var r = world.mode_rules
	var friendly: bool = world.is_friendly(source, self)
	if r != null and r.damageable_spawn_protection_duration > 0.0:
		if world.now - creation_time < r.damageable_spawn_protection_duration:
			d *= CombatConstants.spawn_damage_scale
		var factor: float = 0.0 if r.b_disable_damage else r.damage_factor * (r.team_damage_factor if friendly else 1.0)
		d *= factor
	# Fall (UDamageableComponent::ModifyDamage rva=0x1489be0, `if (EVar2 == Fall)`): d x FallDamageModifier (the
	# character's ReceivedFallDamageModifier +0x9fc) only - no type / DamageModifier / team terms
	if type == DAMAGE_FALL:
		d *= character.received_fall_damage_modifier
	else:
		if type == DAMAGE_RANGED:
			d *= character.received_ranged_damage_modifier
		d *= character.received_damage_modifier
		if r != null and friendly:
			d *= character.received_team_damage_modifier
	var absorption := character.received_damage_absorption
	if d > 0.0:
		d = maxf(d - absorption, 0.0)
	elif d < 0.0:
		d = minf(d + absorption, 0.0)
	if d > 0.0:
		d = maxf(d, CombatConstants.min_damage)
		var max_damage := character.received_damage_max
		if max_damage > 0.0 and d > max_damage:
			d = max_damage
	return minf(d, float(health))

# from UDamageableComponent::OnPostTakeDamage rva=0x1490ad0 (disassembly 0x141490be8..0x141490cd2):
#   NewValue = int(float(StatValue) - Damage)   cvtdq2ps / subss / cvttss2si: truncation toward zero
#   NewValue < 0 -> 0
#   NewValue == 0 with bHasLastChance (+0x9f0) on a Melee/Ranged hit -> NewValue = max(LastChanceHealAmount +0x9f4, 1),
#     bHasLastChance = false
#   SetStatValue(NewValue) (UStatComponent::SetStatValue 0x141515c50); bWillKill = (StatValue == 0)
#   bWillKill with bWillBleedOutOnKill (+0x9b2) and !bIsBleedingOut (+0x9b8) asks CanBleedOutFromHit (not ported:
#     both flags are false in the constructors); otherwise AActor::SetCanBeDamaged(false) and the kill path.
func _post_take_damage(applied: float, type: int) -> void:
	var nv := int(float(health) - applied)
	if nv < 0:
		nv = 0
	if nv == 0 and has_last_chance and (type == DAMAGE_MELEE or type == DAMAGE_RANGED):
		nv = maxi(character.last_chance_heal_amount, 1)
		has_last_chance = false
	var old := health
	_set_health(nv)
	# SetStatValue(NewValue, bReplicate true) -> UHealthStatComponent::SetStatValue_Internal rva=0x14d5cf0 writes the
	# ReplicatedHealth byte (WriteReplicatedStat) when the value changed
	if net_rep != null and health != old:
		net_rep.write_replicated_health()

# UHealthStatComponent::SetStatValue_Internal rva=0x14d5cf0: UStatComponent clamp (< Min -> Min, >= Max -> Max), the
# replicated health byte, then OnDied (+delegate) is broadcast when the value goes from > 0 to < 1.
func _set_health(nv: int) -> void:
	var old := health
	var mn := health_stat.min_value
	var mx := health_stat.max_value
	health = mn if nv < mn else (nv if nv < mx else mx)
	if old > 0 and health < 1 and not dead:
		dead = true
		# UStaminaStatComponent::OnCharacterDied rva=0x1508110 (bound to AAdvancedCharacter OnCharacterDied +0x530 by
		# UStaminaStatComponent::OnRegister rva=0x150dee0): bIsRegenerable = false (the out-of-breath sound is not ported)
		stamina_regenerable = false
		world.trace_event(name + " died")
		world.emit_event({"kind": "died", "who": name})

# AMordhauCharacter health byte = RoundToInt(100 x (StatValue - Min) / (Max - Min)), 1 when that rounds to 0 while
# StatValue != 0 (UHealthStatComponent::SetStatValue_Internal rva=0x14d5cf0)
func health_byte() -> int:
	var mn := float(health_stat.min_value)
	var mx := float(health_stat.max_value)
	var fraction := 0.0
	if absf(mx - mn) > CombatConstants.small_number:			# 1e-8
		fraction = clampf((float(health) - mn) / (mx - mn), 0.0, 1.0)
	elif float(health) >= mx:
		fraction = 1.0
	var percent := int(round(fraction * 100.0))		# 100: percent byte
	return 1 if percent == 0 and health != 0 else percent

# One step for this fighter = AMordhauCharacter::LODTick rva=0x154c390, in the order that function runs:
#  1. AAdvancedCharacter::LODTick rva=0x14887b0 (called first) broadcasts OnLODTick (+0x5b8) before anything else;
#     UMotionSystemComponent::OnRegister rva=0x14cde00 bound UMotionSystemComponent::OnLODTick rva=0x14cb3e0 to it,
#     which runs UMordhauMotion::Tick while !bIsDead (+0x504, disassembly 0x1414cb4a7). The motion system has no
#     tick of its own: its constructor rva=0x14afca0 clears bCanEverTick (PrimaryComponentTick bitfield +0x3a & 0xfd).
#  2. The buffered parry: when not wanting a block but holding block inside a UFlinchMotion (StaticClass
#     0x1416920b0) with a bIsParryHeld (+0x1a0c) weapon, bWantsBlock (+0x1029) is set; then while bWantsBlock:
#     bWantsBlock = !RequestParry(Regular, bAllowFTP true) (disassembly 0x14154c937..0x14154ca3c; the branch runs
#     for locally controlled characters, which every fighter here stands for).
#  3. Stamina: StopStaminaRegen (vcall +0x990) while the current motion has bBlocksRegen (+0x88), then the stamina
#     component's TickStat (vcall +0x410: UStaminaStatComponent::TickStat 0x1519900 -> UStatComponent::TickStat).
# Every character ticks in TG_PrePhysics: FTickFunction's constructor (0x1434cd0a0, `mov word [rcx+8], 0`) and
# AActor::InitializeDefaults (`mov byte [rcx+0x30], 0`) set TickGroup 0 and no character constructor changes it.
# Tracing runs later in the frame, in TG_PostPhysics (late_tick).
func tick(dt: float) -> void:
	if motion != null and not dead:
		motion.tick(dt)
	if not wants_block and holding_block and motion is FlinchMotion and weapon != null and weapon.b_is_parry_held:
		wants_block = true
	if wants_block:
		wants_block = not request_parry(CombatEnums.BlockType.REGULAR, true)
	if motion != null and motion.b_blocks_regen:
		stop_stamina_regen(0.0)
	_tick_stamina_regen()

# The late tick. ULateTickComponent's constructor rva=0x14af270 sets bCanEverTick and TickGroup (+0x38) = 4 =
# TG_PostPhysics (5th name of the ETickingGroup UHT name table "TG_PrePhysics, TG_StartPhysics, TG_DuringPhysics,
# TG_EndPhysics, TG_PostPhysics, ..." at exe file offset 0x4b1c920); its TickComponent rva=0x14d96f0 calls
# AAdvancedCharacter::LateTick (vcall +0x938) -> OnLateTick (+0x5d0) -> UMotionSystemComponent::OnLateTick
# rva=0x14cc120 -> UMordhauMotion::LateTick. UAttackMotion::OnLateTick_Implementation rva=0x1631f00: while an
# attack motion is current, the weapon prepares its tracers (weapon vtable +0x7c0 PrepareForTracing); tracing itself
# runs in Release (MeleeHit.trace_and_process).
func late_tick() -> void:
	if dead:
		return
	if motion is AttackMotion:
		tracer.prepare_for_tracing()

# ---- input ------------------------------------------------------------------------------------------------------
# Attack: AMordhauCharacter::RequestAttack rva=0x15644c0 -> UMotionSystemComponent::RequestAttack rva=0x14d0df0 ->
# RightHandEquipment, else LeftHandEquipment, ->RequestAttack (Blueprint event thunk 0x16b09b0; only BP_Instrument
# overrides it) -> AMordhauWeapon::RequestAttack_Implementation rva=0x163b700:
#   CanPerformAttack(Move) false -> try the mirrored move (RightStrike <-> LeftStrike, Stab <-> AltStab; any other
#   move gives up); still false -> nothing. Then Motion (+0xe8) ->ProcessAttack(Move, Angle) (thunk 0x16d69d0).
func request_attack(m: int, angle: float) -> void:
	if dead or motion == null:
		return
	if not can_perform_attack(m):
		match m:
			CombatEnums.Move.RIGHT_STRIKE: m = CombatEnums.Move.LEFT_STRIKE
			CombatEnums.Move.LEFT_STRIKE: m = CombatEnums.Move.RIGHT_STRIKE
			CombatEnums.Move.STAB: m = CombatEnums.Move.ALT_STAB
			CombatEnums.Move.ALT_STAB: m = CombatEnums.Move.STAB
			_: return
		if not can_perform_attack(m):
			return
	motion.process_attack(m, angle)

# The preset attack inputs UMotionSystemComponent::Request{Right,Left}{,Upper,Lower}Strike / Request{Right,Left}Stab
# (RequestRightStrike 0x14d18d0, RequestRightUpperStrike 0x14d1910, RequestRightLowerStrike 0x14d1850,
# RequestLeftStrike 0x14d14b0, RequestLeftUpperStrike 0x14d14f0, RequestLeftLowerStrike 0x14d1430, RequestRightStab
# 0x14d1890, RequestLeftStab 0x14d1470): Move RightStrike 0 / LeftStrike 1 / Stab 2 / AltStab 3, angle 0 / upper -57.5 /
# lower 60.0 (immediates in each body), through UMotionSystemComponent::AdjustPresetAttackAngleRequest, then
# AMordhauEquipment::RequestAttack of RightHandEquipment (else LeftHandEquipment) = request_attack.
# AdjustPresetAttackAngleRequest rva=0x14b2d80: without the mouse-flip console variable (== 1) the side is flipped
# (FlipSide) when AMordhauCharacter bWantsFlipAttackSide (+0xd78, `cmp byte ptr [rbx + 0xd78], 0` at 0x1414b2e71) is
# set; that is `preset_flip` here. The cvar branch (player controller angling vector) is input-device behaviour, not ported.
var preset_flip := false		# AMordhauCharacter +0xd78 bWantsFlipAttackSide

const PRESET_UPPER_ANGLE := -57.5	# RequestRightUpperStrike / RequestLeftUpperStrike immediate
const PRESET_LOWER_ANGLE := 60.0	# RequestRightLowerStrike / RequestLeftLowerStrike immediate

func request_preset_attack(m: int, angle: float) -> void:
	if preset_flip:
		m = CombatEnums.flip_side(m)
	request_attack(m, angle)

# from UMotionSystemComponent::CanInitiateMotion rva=0x14b4ba0: no motion -> false; ask the current motion
# (CanInitiateMotion); if it refuses, bAttemptCancel is set and the motion IsA UAttackMotion: ProcessFeint, then ask the
# (possibly new) current motion again
func can_initiate_motion(new_kind: String, attempt_cancel: bool) -> bool:
	if motion == null:
		return false
	var ok := motion.can_initiate_motion(new_kind)
	if not ok and attempt_cancel and motion is AttackMotion:
		motion.process_feint()
		ok = motion.can_initiate_motion(new_kind)
	return ok

# from UEquipmentSystemComponent::CheckCanEquipAlt rva=0x14b50e0 (disasm 0x1414b5129..0x1414b5194): the equipment has
# bHasAlternateMode (+0xd23); on a CurrentVehicle (+0x10c0) that is a horse (+0x171) / ladder (+0x170) it also needs
# bSecondCanEquipOnHorse (+0xc73) / bSecondCanEquipOnLadder (+0xc74) (no vehicles here); with an arm disabled
# (bIsLeftArmDisabled +0xd40 / bIsRightArmDisabled +0xd41) the alternate mode must not need it: refused when the arm on
# its side (bSecondIsRightHanded +0x56a: right, else left) is disabled or it is bSecondIsTwoHanded (+0x56c)
func check_can_equip_alt(e: EquipmentDef) -> bool:
	if e == null or not e.b_has_alternate_mode:
		return false
	if b_is_left_arm_disabled or b_is_right_arm_disabled:
		var side_disabled := b_is_right_arm_disabled if e.b_second_is_right_handed else b_is_left_arm_disabled
		if side_disabled or e.b_second_is_two_handed:
			return false
	return true

# AMordhauCharacter::RequestFeint rva=0x15646b0: the current motion's ProcessFeint
func request_feint() -> void:
	if not dead:
		motion.process_feint()

# AMordhauCharacter::RequestParry rva=0x1564860 -> UMotionSystemComponent::RequestParry rva=0x14d1590:
# LeftHandEquipment, else RightHandEquipment, if it is an AMordhauWeapon -> RequestBlock (ProcessEvent with FName
# global NAME_AMordhauWeapon_RequestBlock 0x14572d560; only BP_Instrument and BP_TargeShield_DIH override it) ->
# AMordhauWeapon::RequestBlock_Implementation rva=0x163b7b0:
#   needs bCanBlock (+0xef8) and, on foot, bCanBlockOnFoot (+0xef9); ok = Motion->ProcessBlock(BlockType)
#   (thunk 0x16d6a20); if !ok, bAllowFTP and the current motion is a UAttackMotion (StaticClass 0x14167dbd0):
#   Motion->ProcessFeint() (thunk 0x16d6a70), then ok = (new) Motion->ProcessBlock(BlockType). Returns ok.
# Bots call this directly (UBTTask_MeleeDefend: RequestParry(type, true)); players go through block_pressed.
# The equipment asked is the left-hand item when it IsA AMordhauWeapon (a shield), else the right-hand weapon; its
# own bCanBlock / bCanBlockOnFoot apply (BP_PaviseShield has bCanBlock false). A shield first runs
# AMordhauShield::RequestBlock_Implementation rva=0x15f16c0: if the current motion is a UParryMotion with
# bIsShieldWall (+0x46a), then in the Parry stage (+0x53a == 0) and not yet bRequestedDrop (+0x533):
# ServerDropParry (0x1416a5c10) and bRequestedDrop = 1; either way it returns false without blocking. Otherwise it
# falls through to AMordhauWeapon::RequestBlock_Implementation.
func request_parry(bt: int, allow_ftp := true) -> bool:
	if dead or motion == null or weapon == null:
		return false
	var use_left := _left_is_weapon()
	if use_left and left_hand.is_shield and motion is ParryMotion and motion.b_is_shield_wall:
		if motion.stage == CombatEnums.ParryStage.PARRY and not motion.b_requested_drop:
			server_drop_parry()
			if motion is ParryMotion:
				motion.b_requested_drop = true
		return false
	var can_block: bool = left_weapon.b_can_block if use_left else weapon.b_can_block
	var on_foot: bool = left_weapon.b_can_block_on_foot if use_left else weapon.b_can_block_on_foot
	if not can_block or not on_foot:
		return false
	var ok := motion.process_block(bt)
	if ok or not allow_ftp or not (motion is AttackMotion):
		return ok
	motion.process_feint()
	return motion.process_block(bt)

# AMordhauCharacter::BlockPressed rva=0x1531650: bIsHoldingBlock = true; bWantsBlock = !RequestParry(Regular, true)
# (retried every tick, see tick). The block type argument stays for callers that pass AltRegular.
func block_pressed(bt := CombatEnums.BlockType.REGULAR) -> void:
	holding_block = true
	wants_block = not request_parry(bt, true)

# AMordhauCharacter::BlockReleased rva=0x15316a0: bWantsBlock = false, bIsHoldingBlock = false. A held parry then ends
# through UParryMotion::OnTick_Implementation -> ServerDropParry (parry_motion.gd).
func release_block() -> void:
	wants_block = false
	holding_block = false

# AMordhauWeapon::SwitchMode_Implementation rva=0x1640a00, the fields combat reads: Swap<FAttackInfo> of
# Strike/SecondStrike, Stab/SecondStab, Kick/SecondKick, Couch/SecondCouch; BlockStaminaNegation <->
# SecondBlockStaminaNegation; BlockStaminaClamp <-> SecondBlockStaminaClamp; Length <-> SecondLength;
# WeaponAnimationProfile <-> SecondWeaponAnimationProfile (attack motion classes and parry motion). Sounds, trails,
# dismemberment, supersprint and the mode-switch montage are not combat state. Only this fighter's copies change,
# never the cached class defaults. Not ported: the input/animation that leads to SwitchMode (OnRequestModeSwitch),
# and the tracer's switch to the SecondTraceStart/End sockets (GetTrace_Implementation in alt mode).
const _ATTACK_SWAPS := [["strike", "second_strike"], ["stab", "second_stab"], ["kick", "second_kick"],
	["couch", "second_couch"], ["block_stamina_negation", "second_block_stamina_negation"],
	["block_stamina_clamp", "second_block_stamina_clamp"]]

# UMotionSystemComponent::GetParryMotionClass rva=0x14be9b0: default UParryMotion; the right-hand weapon's
# (+0x11f8) GetRelevantMeleeWeaponAnimationProfile()->ParryMotion if set; then the LEFT-hand equipment's (+0x1200)
# profile ParryMotion if set overrides it (the left hand is checked last). "" = native UParryMotion.
func parry_motion_bp() -> String:
	var bp := CombatData.profile_motions(weapon_equip.weapon_animation_profile).parry_motion if weapon_equip != null else ""
	if left_hand != null:
		var lp := CombatData.profile_motions(left_hand.weapon_animation_profile).parry_motion
		if lp != "":
			bp = lp
	return bp

# UParryMotion::OnBegin_Implementation rva=0x1660f70 (disasm 0x14166112d..0x141661201): WeaponPtr (+0x550) =
# LeftHandEquipment (+0x1200) if it IsA AMordhauWeapon (GetPrivateStaticClass 0x14170dba0; AMordhauShield derives
# from it), else RightHandEquipment (+0x11f8) if it IsA AMordhauWeapon. OnBegin (bIsParryHeld, ParryWindowOffset,
# ParryHeldStaminaDrain) and ReceiveBlock rva=0x166bf90 (BlockStaminaNegation +0x1a38, BlockStaminaClamp +0x1a3c,
# disasm 0x14166c1a0..0x14166c1ba) read that weapon. So a one-hander carried with a buckler parries with the
# buckler's negation (BP_BucklerShield 14), not the sword's.
func parry_weapon() -> WeaponData:
	return left_weapon if _left_is_weapon() else weapon

# the AMordhauWeapon whose WeaponPtr a parry uses, as equipment (path, class)
func parry_weapon_equip() -> EquipmentDef:
	return left_hand if _left_is_weapon() else weapon_equip

func _left_is_weapon() -> bool:
	return left_hand != null and left_hand.is_weapon

# AMordhauCharacter::RequestToggleWeaponMode rva=0x1564a60: LeftHandEquipment (+0x1200) OnRequestModeSwitch first,
# then RightHandEquipment (+0x11f8); the first that returns true ends it. AMordhauShield::
# OnRequestModeSwitch_Implementation rva=0x15e6d70, shield-wall branch: bCanBlock and bAllowShieldWall (+0x1c98) and
# the current motion IsA UIdleMotion (StaticClass 0x141694c90) and no CurrentVehicle -> ok = Motion->ProcessBlock
# (ShieldWall 2); if the motion is now a UAttackMotion and !ok: ProcessFeint, ProcessBlock(2) again; return ok.
# Without bAllowShieldWall its other branches start ranged/reload/equipment-mode-switch motions (not ported, false).
# The right-hand weapon's mode switch is modelled as the immediate SwitchMode swap (switch_mode below).
# UNCONFIRMED: AMordhauWeapon::OnRequestModeSwitch_Implementation's motion (UEquipmentModeSwitchMotion) timing.
func toggle_weapon_mode() -> void:
	if dead or motion == null:
		return
	if _left_is_weapon() and left_hand.is_shield:
		if left_weapon.b_can_block and left_hand.b_allow_shield_wall and motion is CombatMotion.Idle:
			var ok := motion.process_block(CombatEnums.BlockType.SHIELD_WALL)
			if not ok and motion is AttackMotion:
				motion.process_feint()
				ok = motion.process_block(CombatEnums.BlockType.SHIELD_WALL)
			if ok:
				return
	switch_mode()

# AMordhauWeapon::OnRequestModeSwitch_Implementation rva=0x16327a0, bHasAlternateMode branch: check_can_equip_alt, then
# UMotionSystemComponent::CanInitiateMotion(UEquipmentModeSwitchMotion, bAttemptCancel true) before the
# EquipmentModeSwitch NetMotion; a weapon without bHasAlternateMode takes the ranged/reload branches (not ported).
# AMordhauWeapon::OnRequestModeSwitch_Implementation rva=0x16327a0 (bHasAlternateMode branch): CheckCanEquipAlt,
# CanInitiateMotion(UEquipmentModeSwitchMotion, true), then the motion (fp-anim r3: mode_switch_motion.gd; the swap
# itself runs in its FinishSwitch, StartTime + 0.15)
func switch_mode() -> void:
	if weapon == null or not check_can_equip_alt(weapon_equip):
		return
	if not can_initiate_motion("EquipmentModeSwitch", true):
		return
	var m := ModeSwitchMotion.new(self, ModeSwitchMotion.make_def())
	m.b_is_switching_to_alt = not alternate_mode
	_change(m)

# AMordhauCharacter::SwitchModeAndReAttach -> AMordhauWeapon::SwitchMode_Implementation rva=0x1640a00: the swap (called
# by ModeSwitchMotion's FinishSwitch, and directly for a fighter spawned in its alternate mode)
func switch_mode_and_reattach() -> void:
	var w: WeaponData = weapon.duplicate()
	for pair in _ATTACK_SWAPS:
		var first = w.get(pair[0])
		w.set(pair[0], w.get(pair[1]))
		w.set(pair[1], first)
	weapon = w
	weapon_equip = weapon_equip.switched()
	var prof := CombatData.profile_motions(weapon_equip.weapon_animation_profile)
	motion_bps = prof.motions
	parry_bp = parry_motion_bp()
	alternate_mode = not alternate_mode
	trace_event("switch mode -> %s" % ("alternate" if alternate_mode else "primary"))

func trace_event(s: String) -> void:
	world.trace_event(name + " " + s)

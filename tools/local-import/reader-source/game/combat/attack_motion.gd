# attack_motion.gd - UAttackMotion (+ the UStrikeMotion / UStabMotion overrides that change timing or rules).
# Every function names the decompiled function it ports: extract/native/decomp/UAttackMotion.cpp (rva = the "// ... rva="
# header of that function), UStrikeMotion.cpp, UStabMotion.cpp. Field offsets in comments are UAttackMotion.h.
# Constants: MotionDefs.Attack (CombatData.motion_def: native ctor + Blueprint CDO chain; fields cited there).
# Attack values: the FAttackInfo of the attack's weapon (FindWeapon: the right-hand weapon, or the KickWeapon for a
# UKickMotion; WeaponData from UeWeapon).
# Ported beyond the timeline: the AutoBlend windup-offset search (OnBegin; attack_auto_blend.gd, shared with the
# animation layer, runs when MotionSystem.anim_poses is attached), the Wind-up / Release curves, the trace memory.
# Network lag terms: LagInduction / LagReduction are 0 on one machine; game/net fills them (net_rep.attack_lag).
# Not ported (no effect on the state/timer trace): the montage playback itself (has_montage stands for it), angling
# additives, supersprint, turn caps, sounds, StrikeMotion/StabMotion OnTickWindUp (angle cue animation).
# Hit tracing and damage: melee_hit.gd.
class_name AttackMotion
extends CombatMotion

var native := "UAttackMotion"	# UStrikeMotion / UStabMotion / UKickMotion ... (MotionDefs.Base.native)
var weapon: WeaponData = null	# +0x10c8 Weapon (MotionSystem.find_weapon = FindWeapon)
var ad: MotionDefs.Attack		# this class's defaults (= def, typed)
var ai: AttackInfo				# +0xe38 AttackInfo, a copy of the weapon's, then ModifyAttackInfo'd
var type := 0					# +0x108d EAttackType
var move := 0					# +0x108e EAttackMove
var stage := 0					# +0x10e9 EAttackStage
var windup_end := 0.0			# +0x1090
var release_end := 0.0			# +0x1094
var lag_induction := 0.0		# +0x1098 (0 offline)
var lag_reduction := 0.0		# +0x109c (0 offline)
var angle_target := 0.0			# +0x1080
var last_release_norm := 0.0	# +0x1084 LastReleaseNormalizedTime
var last_windup_norm := 0.0		# +0x1088 LastWindupNormalizedTime
var b_has_queued_move := false	# +0x10e1
var queued_move := 0			# +0x10e8
var queued_angle := 0.0			# +0x10e4
var b_has_hit := false			# +0x10ea
var b_has_hit_friendly := false	# +0x108c bHasHitFriendly
var b_has_chambered := false	# +0x10f4
var b_has_considered_combo := false	# +0x1070
var b_is_combo_from_miss := false	# +0xa93
var b_riposte_ate_feint_input := false	# +0xb99
var coming_from_move := 0		# +0xe2a ComingFromMove
var previous_last_attack = null	# +0x10d0 PreviousLastAttackMotion
var first_hit_time := 0.0		# +0x10ec
var global_damage_modifier := 1.0	# +0xa84 GlobalDamageModifier (ExecuteAttackTracingAndLogic sets 1.0)
var early_release := 0.0		# +0xb48 EarlyRelease (Riposte/PostClash: RiposteEarlyRelease, PrepareAnimationData)
var early_release_tf := 1.0		# +0xb4c EarlyReleaseTimeFactor
var windup_anim_offset := 0.0	# +0x10a4 WindUpAnimationStartTimeOffset (AutoBlend search result, else class default)
var blend_in := 0.0				# +0xc68 BlendIn (AutoBlend result; animation only)
var windup_curve := ""			# +0xdf8 WindUpCurve (object path, "" = none)
var release_curve := ""			# +0xe18 ReleaseCurve
var hit_actors := []			# weapon +0xe98 ActorSetCache: actors already hit this release (MotionSystem.id of each)
var b_has_last_trace := false	# +0xa2 bHasLastTrace
var last_trace_start := Vector3.ZERO	# +0xa4 LastTraceStart (weapon CurrentTraceStart of the last Release tick)
var last_trace_end := Vector3.ZERO		# +0xb0 LastTraceEnd
var bounce_montage := ""		# +0xae0 BounceMontage: the class default, a kick's equipment KickBounce (ModifyAttackInfo)
var b_is_air_kick := false		# UKickMotion +0x1114 bIsAirKick

func _init(owner_sys: MotionSystem, definition: MotionDefs.Base) -> void:
	super(owner_sys, definition)
	ad = definition as MotionDefs.Attack

func kind() -> String:
	return "Attack"

# from UAttackMotion::GetMovementRestriction rva=0x16245a0 (disasm 0x1416245a0..0x1416245c2): bIsComboFromMiss (+0xa93)
# -> 1 (PartialSprint); Recovery (Stage +0x10e9 == 2) -> !bHasHit (+0x10ea); else 0 (None). The class field is not read.
# UKickMotion::GetMovementRestriction rva=0x165cbe0: an airborne owner (vcall +0x9e0 = AAdvancedCharacter::IsAirborne,
# types/AAdvancedCharacter.h slot 316) -> JumpKickAirMovementRestriction (+0x110c), else the motion's field (+0x61).
func get_movement_restriction() -> int:
	if native == "UKickMotion":
		if sys.airborne:
			return (ad as MotionDefs.Kick).jump_kick_air_movement_restriction
		return movement_restriction
	if b_is_combo_from_miss:
		return CombatEnums.MovementRestriction.PARTIAL_SPRINT
	if stage == CombatEnums.Stage.RECOVERY:
		return CombatEnums.MovementRestriction.NONE if b_has_hit else CombatEnums.MovementRestriction.PARTIAL_SPRINT
	return CombatEnums.MovementRestriction.NONE


func is_strike_class() -> bool:
	return native == "UStrikeMotion"

func is_stab_class() -> bool:
	return native == "UStabMotion"

# Montage (+0x10b0) != null: PlayAttackAnim rva=0x1636d90 stores AAdvancedCharacter::PlayAnim's montage there. Taken
# as set whenever the attack has something to play (its Animation, or the KickWeapon's kick montages for a UKickMotion).
# UNCONFIRMED: that PlayAnim returns a montage on a dedicated server (engine anim-instance behaviour). UBlockedMotion
# reads it (OnTick returns at once without FromAttackMontage; OnBegin's kick hit-stop rate).
func has_montage() -> bool:
	return ad.animation != "" or CombatData.is_class_of(native, "UKickMotion")

# from UAttackMotion::OnBegin_Implementation rva=0x162eda0 (timeline part) + ModifyAttackInfo + ComputeWindup
func on_begin() -> void:
	native = def.native
	weapon = sys.find_weapon(native)
	sys.tracer.actor_ignore_cache = [sys.id]	# OnBegin: ActorIgnoreCache = [owner]
	var net = sys.net
	var last = sys.last_attack_motion				# MotionSystem+0x108 LastAttackMotion
	if last != null:
		previous_last_attack = last
		coming_from_move = last.move
	type = CombatEnums.hi(net.param0)			# Param0 = SetHI(EAttackType) | SetLO(EAttackMove)
	var extra := 0.0
	if type == CombatEnums.AttackType.MISS_COMBO:
		type = CombatEnums.AttackType.COMBO
		# authority: the previous attack hit after the combo was queued -> dynamic param bit 4
		# (UAttackMotion::OnBegin_Implementation, disassembly 0x14162ef3c..0x14162ef59: PreviousLastAttackMotion->bHasHit, NetMotion.MotionDynamicParam | 4)
		var late := CombatEnums.DYN_PARENT_HIT_LATE
		if previous_last_attack != null and previous_last_attack.b_has_hit and (net.dynamic_param | late) != net.dynamic_param:
			sys.net.dynamic_param = net.dynamic_param | late
		if (sys.net.dynamic_param & late) == 0:
			b_is_combo_from_miss = true
	elif type == CombatEnums.AttackType.POST_CLASH_SLOW:
		type = CombatEnums.AttackType.POST_CLASH
		extra = ad.clash_on_parry_follow_up_windup
	move = CombatEnums.lo(net.param0)
	# angle byte -> [-1, 1], clamped to AnglingLimits (+0xf0); x CombatConstants.byte_to_unit (1/255)
	var a := clampf(float(net.param1) * CombatConstants.byte_to_unit, 0.0, 1.0) * 2.0 - 1.0	# x 1/255 as stored
	var lim := ad.angling_limits
	angle_target = a if a >= lim.x else lim.x
	if a >= lim.y:
		angle_target = lim.y
	# Morph: the attack being morphed out of pays its MorphCost (authority only; we are the authority)
	if type == CombatEnums.AttackType.MORPH and last != null:
		sys.offset_stamina(-last.ai.morph_cost)
	ai = CombatData.attack_info_for(weapon if weapon != null else sys.weapon, move).duplicate(true)
	_prepare_curves()
	var st := AttackAutoBlend.settings(ad, type)
	early_release = st.early_release
	early_release_tf = st.early_release_tf
	windup_anim_offset = ad.windup_animation_start_time_offset
	bounce_montage = ad.bounce_montage
	modify_attack_info()
	ai.windup = compute_windup() + extra - float(net.param2) * CombatConstants.net_lag_unit_s
	if sys.net_rep != null:		# game/net: EstimatedNetworkDelay -> LagReduction / LagInduction / MissRecovery
		sys.net_rep.attack_lag(self)
	ai.windup = lag_reduction + lag_induction + ai.windup
	windup_end = ai.windup + start_time
	release_end = windup_end + ai.release
	end_time = release_end + ai.miss_recovery
	# AutoBlend search (OnBegin, after "bUseAutomaticBlendIn"): needs the pose shown now and the attack clip's poses,
	# which only the animation layer has: sys.anim_poses (MotionSystem.AnimPoses, optional). Without it the class
	# default offset stays (headless rules).
	var ap: MotionSystem.AnimPoses = sys.anim_poses
	if st.auto and ai.windup > 0.0 and ap != null:
		var clip: String = ap.clip.call(self)
		if clip != "":
			var r := AttackAutoBlend.search(ad, st.steps, start_time, windup_end, release_end, sys.tracer.length,
				ap.now.call(), func(tt): return ap.at.call(clip, tt))
			blend_in = minf(r.blend_in, ai.windup)
			windup_anim_offset = r.offset

# Curve selection of UAttackMotion::PrepareAnimationData_Implementation rva=0x1637140: Combo -> WindUpCurve =
# ComboWindUpCurve.Get(bFirstPerson) (+0xe00); Morph -> WindUpCurve = MorphWindupCurve (+0xba8); Riposte ->
# ReleaseCurve = RiposteReleaseCurve (+0xe20); otherwise the class's own WindUpCurve / ReleaseCurve.
# Perspective: ThirdPerson (the simulation has no first-person viewer; UMordhauMotion::GetIsFirstPerson not ported).
func _prepare_curves() -> void:
	windup_curve = ad.windup_curve
	release_curve = ad.release_curve
	if type == CombatEnums.AttackType.COMBO:
		windup_curve = ad.combo_windup_curve
	elif type == CombatEnums.AttackType.MORPH:
		windup_curve = ad.morph_windup_curve
	elif type == CombatEnums.AttackType.RIPOSTE:
		release_curve = ad.riposte_release_curve

# from UAttackMotion::ComputeWindup_Implementation rva=0x1618de0: return AttackInfo.Windup
func compute_windup() -> float:
	return ai.windup

# from UAttackMotion::ModifyAttackInfo_Implementation rva=0x162d870 (+ UStabMotion override rva=0x165d830)
func modify_attack_info() -> void:
	var ch: CharacterData.Character = sys.character			# AMordhauCharacter Melee*Modifier
	var w := ai.windup * ch.melee_windup_modifier
	if type == CombatEnums.AttackType.COMBO:
		if coming_from_move != CombatEnums.Move.KICK:
			w = ch.melee_combo_extra_windup_modifier * ai.combo_windup_increase + w
			if b_is_combo_from_miss:
				w = w + ai.miss_combo_extra_windup_increase
	elif type == CombatEnums.AttackType.MORPH:
		w = w * ad.morph_windup_modifier
	elif type == CombatEnums.AttackType.RIPOSTE:
		w = w * ad.riposte_windup_modifier
	ai.windup = w
	# perk 6 (TurnCaps), vehicle bCanCombo: not modelled (no perks, no vehicles)
	if b_is_combo_from_miss:
		ai.miss_stamina_cost = ad.miss_twice_stamina_cost_multiplier * ai.miss_stamina_cost
	ai.release = ch.melee_release_modifier * ai.release
	ai.miss_recovery = ch.melee_miss_recovery_modifier * ai.miss_recovery
	if is_stab_class():
		ai.release = weapon.stab_release_modifier * ai.release	# Weapon (+0x10c8) +0xf38 StabReleaseModifier
	if ad is MotionDefs.Kick:
		_modify_kick_attack_info(ch)

# from UKickMotion::ModifyAttackInfo_Implementation rva=0x165d6e0 (decomp UKickMotion.cpp 111-177), after the base:
#   BounceMontage = KickBounce of LeftHandEquipment, else RightHandEquipment, else this attack's Weapon (KickWeapon)
#   Combo from a kick: Windup -= MeleeComboExtraWindupModifier x ComboWindupIncrease
#   Windup += 0.3 (CombatConstants.kick_windup_extra; the kick's NetMotion lag subtracts up to 0.3 again: kick_lag_window)
#   GetArmorTierForBone(NAME_RightLeg) (vcall +0x958) == 3: every Damage entry x KickDamageModifierTier3Legs (the
#     kicker's own right-leg armour)
#   IsAirborne() (vcall +0x9e0) and AirborneTime < MaxAirborneTimeForJumpKickAnim: bIsAirKick, StaminaDrain =
#     JumpKickStaminaDrain, Windup += JumpKickExtraWindup
# A left-hand equipment that is not an AMordhauWeapon has no WeaponData here (no such loadout in this simulation).
func _modify_kick_attack_info(ch: CharacterData.Character) -> void:
	var kd := ad as MotionDefs.Kick
	var src: WeaponData = sys.left_weapon if sys.left_weapon != null else (sys.weapon if sys.weapon != null else weapon)
	if src != null:
		bounce_montage = src.kick_bounce
	if type == CombatEnums.AttackType.COMBO and coming_from_move == CombatEnums.Move.KICK:
		ai.windup = ai.windup - ch.melee_combo_extra_windup_modifier * ai.combo_windup_increase
	ai.windup = ai.windup + CombatConstants.kick_windup_extra
	if MeleeDamage.armor_tier(sys, CombatConstants.kick_tier_bone) == 3:
		for i in ai.damage.size():
			ai.damage[i] = kd.kick_damage_modifier_tier3_legs * ai.damage[i]
	if sys.airborne and sys.airborne_time < kd.max_airborne_time_for_jump_kick_anim:
		b_is_air_kick = true
		ai.stamina_drain = kd.jump_kick_stamina_drain
		ai.windup = kd.jump_kick_extra_windup + ai.windup

# from UAttackMotion::OnTick_Implementation rva=0x16328c0 (state/timer part)
func on_tick(_dt: float) -> void:
	var t := now()
	if stage == CombatEnums.Stage.WINDUP:
		# riposte / chambered windup: owner's EasyParryUntilTime = now + CombatConstants.easy_parry_window (0.5)
		if type == CombatEnums.AttackType.RIPOSTE or b_has_chambered:
			sys.easy_parry_until_time = t + CombatConstants.easy_parry_window
		if ad.min_windup_time_before_morphing + start_time < t and b_has_queued_move:
			process_attack(queued_move, queued_angle)	# UMordhauMotion::ProcessAttack -> virtual dispatch
			if sys.motion != self:
				return
		if windup_end < t:
			b_has_queued_move = false
			stage = CombatEnums.Stage.RELEASE
			hit_actors.clear()		# TSet<AActor*>::Empty(weapon + 0xe98)
	if release_end - lag_induction < t and not b_has_considered_combo and ai.b_can_combo:
		b_has_considered_combo = true
		if b_has_queued_move and sys.stamina > 0 and (b_has_hit or ai.b_can_miss_combo):
			var combo_type := CombatEnums.AttackType.COMBO if b_has_hit else CombatEnums.AttackType.MISS_COMBO
			sys.assign_net_attack_motion(combo_type, queued_move, queued_angle)
			if sys.motion != self:
				return
	if stage == CombatEnums.Stage.RELEASE:
		sys.easy_parry_until_time = 0.0
		if release_end < t:
			b_has_queued_move = false
			enter_recovery()
			if not b_has_hit:
				sys.offset_stamina(int(-ai.miss_stamina_cost))
	if stage == CombatEnums.Stage.WINDUP or stage == CombatEnums.Stage.RELEASE:
		_update_normalized_times(t)

func _update_normalized_times(t: float) -> void:
	var early_end := (release_end - windup_end) * early_release * early_release_tf + windup_end
	var ws := start_time			# bIncludeMissingDeltaTime is off offline (bInitiatedLocally path not modelled)
	var wn := 1.0
	if t < windup_end:
		wn = 0.0 if t <= ws else ((t - ws) / (windup_end - ws) if ws < windup_end else 1.0)
	last_windup_norm = wn
	if stage == CombatEnums.Stage.WINDUP:
		last_windup_norm = get_smoothed_windup_normalized_time(wn)
		# OnTick_Implementation rva=0x16328c0: WindUpCurve (+0xdf8) applied in place to LastWindupNormalizedTime
		if windup_curve != "":
			last_windup_norm = CombatData.curve_value(windup_curve, last_windup_norm)
	elif stage == CombatEnums.Stage.RELEASE:
		var er := early_release
		if t < early_end:
			var r := 0.0
			if windup_end < t:
				r = (t - windup_end) / (early_end - windup_end) if windup_end < early_end else 1.0
			last_release_norm = er * minf(r, 1.0)
		else:
			var r2 := 1.0
			if t < release_end:
				if early_end < t:
					if early_end < release_end:
						r2 = (t - early_end) / (release_end - early_end)
				else:
					r2 = 0.0
			last_release_norm = (1.0 - er) * r2 + er
		# OnTick_Implementation rva=0x16328c0 (Ghidra C line "if (param_1[0x1c3] != 0) *pfVar19 =
		# UCurveFloat::GetFloatValue(param_1[0x1c3], *pfVar19)", exe 0x1416331c9): ReleaseCurve (+0xe18) applied in place
		# to LastReleaseNormalizedTime, so IsInEarlyRelease compares the curved value
		if release_curve != "":
			last_release_norm = CombatData.curve_value(release_curve, last_release_norm)

# from UAttackMotion::GetSmoothedWindUpNormalizedTime rva=0x1628bf0 (shared with the animation layer)
func get_smoothed_windup_normalized_time(x: float) -> float:
	return AttackAutoBlend.smoothed_windup(ad, start_time, windup_end, release_end, windup_anim_offset, x)

# from UAttackMotion::EnterRecovery rva=0x161b7b0 (state part; it also calls MaybeStoreLastTraceInGameStateMemory)
func enter_recovery() -> void:
	maybe_store_last_trace()
	if not ad.b_do_not_make_recovery_flinchable:
		b_is_flinchable = true
	stage = CombatEnums.Stage.RECOVERY

# from UAttackMotion::OnLeave_Implementation rva=0x16323e0 (trace-memory part): leaving during Release stores the
# last trace too (`Stage == 1 && (MaybeStoreLastTraceInGameStateMemory(this), !bHasHit)`, the miss-cost test)
func on_leave(_interrupted: bool) -> void:
	if stage == CombatEnums.Stage.RELEASE:
		maybe_store_last_trace()

# from UAttackMotion::MaybeStoreLastTraceInGameStateMemory rva=0x162d740: if bHasLastTrace, append
# FLineTraceMemoryEntry{LastTraceStart, LastTraceEnd, DestroyTime = now + TraceMemoryStayDuration (+0xa58), Owner}
# to AMordhauGameState::AttackTracesMemory (+0x530; types/FLineTraceMemoryEntry.h). Parry motions test it
# (parry_motion.gd on_tick) to tell a miss-parry from a parry with nothing near.
func maybe_store_last_trace() -> void:
	if not b_has_last_trace:
		return
	sys.world.attack_traces_memory.append({"start": last_trace_start, "end": last_trace_end,
		"destroy_time": now() + ad.trace_memory_stay_duration, "owner": sys.id})

# from UAttackMotion::GetEarlyReleaseDuration rva=0x1622870
func get_early_release_duration() -> float:
	return (release_end - windup_end) * early_release * early_release_tf

# from UAttackMotion::IsInEarlyRelease rva=0x162c110; UStrikeMotion::IsInEarlyRelease rva=0x165d610 adds the
# look-up terms:
#   look_up   = max(GetRawLookUpValue(), 0) / LookUpLimit
#               (AAdvancedCharacter::GetRawLookUpValue rva=0x1485240 returns LookUpValue +0x520, degrees;
#                LookUpLimit +0x8a0 is the character's, BP_MordhauCharacter CDO 55)
#   reversed  = -AngleTarget (+0x1080)
#   EarlyRelease + (min(reversed, 0) + 1) x look_up x ExtraEarlyReleaseForLookUpNonUndercuts
#                + max(reversed, 0) x look_up x ExtraEarlyReleaseForLookUpOverheads
func is_in_early_release() -> bool:
	var er := early_release
	if is_strike_class():
		var limit: float = sys.character.look_up_limit
		var look_up := maxf(sys.look_up_value, 0.0) / limit
		var reversed := -angle_target
		var non_undercut_term := (minf(reversed, 0.0) + 1.0) * look_up * ad.extra_early_release_for_look_up_non_undercuts
		var overhead_term := maxf(reversed, 0.0) * look_up * ad.extra_early_release_for_look_up_overheads
		er = non_undercut_term + overhead_term + er
	return stage == CombatEnums.Stage.RELEASE and last_release_norm < er

# from UAttackMotion::IsInLiveRecovery rva=0x162c130: (Stage & 0xfd) == 0
func is_in_live_recovery() -> bool:
	return (stage & 0xfd) == 0

# from UAttackMotion::ConvertToCombo rva=0x1619d40 (returns bool, writes the move through an out parameter)
class ComboMove:
	var ok := false
	var move := 0
	func _init(o: bool, m: int) -> void:
		ok = o
		move = m

func convert_to_combo(req: int) -> ComboMove:
	var cur := move
	if cur == req:
		if req > 3:
			return ComboMove.new(false, req)
		return ComboMove.new(true, CombatEnums.flip_side(req))
	if (cur > 3) or (req > 3):
		return ComboMove.new(true, req)
	if CombatEnums.is_left(cur) != CombatEnums.is_left(req):
		return ComboMove.new(true, req)
	return ComboMove.new(true, CombatEnums.flip_side(req))

# from UAttackMotion::CanMorphInto rva=0x1616820 (class != own class); UStrikeMotion::CanMorphInto rva=0x164ec90
# also refuses any UStrikeMotion class; UStabMotion::CanMorphInto rva=0x164ebe0 is the same shape and refuses
# UStabMotion classes (its call at 0x14164ec2f is UStabMotion::StaticClass, labels_all.tsv)
func can_morph_into(target: MotionDefs.Base) -> bool:
	if target == null:
		return false
	var tn := target.native
	if is_strike_class() and tn == "UStrikeMotion":
		return false
	if is_stab_class() and tn == "UStabMotion":
		return false
	return target.path != bp

# from UStrikeMotion::ModifyRequestedMorphAttack rva=0x165d960 / UStabMotion rva=0x165d900; base rva=0x7bf350 no-op.
# (move and angle are in/out parameters there)
class MorphRequest:
	var move := 0
	var angle := 0.0
	func _init(m: int, a: float) -> void:
		move = m
		angle = a

func modify_requested_morph_attack(m: int, angle: float) -> MorphRequest:
	if not (is_strike_class() or is_stab_class()):
		return MorphRequest.new(m, angle)
	if not (CombatEnums.is_strike(m) or CombatEnums.is_stab(m)):
		return MorphRequest.new(m, angle)
	if CombatEnums.is_left(m) != CombatEnums.is_left(move):
		m = CombatEnums.flip_side(m)
	if is_strike_class() and CombatEnums.is_stab(m):
		angle = angle_target * CombatConstants.morph_stab_angle_scale
	return MorphRequest.new(m, angle)

# from UStrikeMotion::ModifyRequestedBlockType rva=0x165d8c0 / UStabMotion rva=0x165d880; base rva=0x154fc80 returns it
func modify_requested_block_type(bt: int) -> int:
	if is_strike_class():
		if CombatEnums.is_left(move):
			return 0 if bt == 1 else bt
		return 1 if bt == 0 else bt
	if is_stab_class():
		if CombatEnums.is_left(move):
			return 1 if bt == 0 else bt
		return 0 if bt == 1 else bt
	return bt

func _queue(m: int, angle: float) -> bool:
	queued_angle = angle
	b_has_queued_move = true
	queued_move = m
	return false

# from UAttackMotion::ProcessAttack_Implementation rva=0x1637cf0
func process_attack(m: int, angle: float) -> bool:
	var t := now()
	if stage == CombatEnums.Stage.RELEASE and ai.b_can_combo:
		var cv := convert_to_combo(m)
		if not cv.ok:
			return false
		if not sys.can_perform_attack(cv.move):
			return false
		if b_has_queued_move:
			return false
		return _queue(cv.move, angle)
	if stage == CombatEnums.Stage.WINDUP:
		var req: MotionDefs.Attack = sys.attack_motion_defaults(m)
		var auto_feint: bool = req.b_can_auto_feint_to_attack	# CDO of the requested class
		var mm := modify_requested_morph_attack(m, angle)
		var tgt: MotionDefs.Attack = sys.attack_motion_defaults(mm.move)
		var morph_ok := false
		var mw := ad.morph_window
		if mw != 0.0:
			var lim := minf(ad.max_morph_total_time - lag_induction, (windup_end - start_time) - mw - lag_induction)
			morph_ok = t < lim + start_time
		if tgt != null and type == CombatEnums.AttackType.REGULAR and can_morph_into(tgt) and morph_ok:
			if sys.stamina < ai.morph_cost:
				return false
			if ad.min_windup_time_before_morphing + start_time < t:
				if mm.move == CombatEnums.Move.KICK:
					sys.next_kick_time = maxf(t + ad.morph_kick_extra_time, sys.next_kick_time)
				sys.assign_net_attack_motion(CombatEnums.AttackType.MORPH, mm.move, mm.angle)
				return sys.motion != self
			return _queue(mm.move, mm.angle)
		# not a morph: feint into the attack if the requested class allows it (bCanAutoFeintToAttack)
		if not auto_feint or not can_morph_into(req) or not process_feint():
			return false
		if sys.motion is FeintedMotion:
			sys.motion.process_attack(m, angle)
		return false
	if t < end_time - ad.recovery_queue_window:
		return false
	return _queue(m, angle)

# from UAttackMotion::ProcessFeint_Implementation rva=0x1638220
# UKickMotion::ProcessFeint_Implementation rva=0x166bd20 is the same body with one extra first test: an air kick
# (bIsAirKick +0x1114) cannot be feinted (returns false, the queued move kept)
func process_feint() -> bool:
	if native == "UKickMotion" and b_is_air_kick:
		return false
	if type == CombatEnums.AttackType.RIPOSTE:
		if not b_riposte_ate_feint_input:
			b_riposte_ate_feint_input = true
			if not ad.b_is_riposte_feintable:
				return true
		elif not ad.b_is_riposte_feintable:
			return false
	var t := now()
	var cost: int = ai.chamber_feint_cost if b_has_chambered else ai.feint_cost
	var can_pay: bool = cost <= sys.stamina
	var in_window := stage == CombatEnums.Stage.WINDUP and not ((windup_end - lag_induction) - ad.feint_window < t)
	if can_pay and in_window:
		var ft := CombatEnums.FeintType.REGULAR
		if type == CombatEnums.AttackType.COMBO:
			ft = CombatEnums.FeintType.COMBO
		elif b_has_chambered:
			ft = CombatEnums.FeintType.CHAMBER
		sys.assign_net_feint(ft, move)
		return true
	b_has_queued_move = false
	return false

# from UAttackMotion::ProcessBlock_Implementation rva=0x16380a0
func process_block(bt: int) -> bool:
	if weapon == null:			# Weapon (+0x10c8) null -> false
		return false
	var t := now()
	if not b_has_hit:
		var riposte_parry := b_riposte_ate_feint_input and type == CombatEnums.AttackType.RIPOSTE and stage == CombatEnums.Stage.WINDUP \
			and not ((windup_end - lag_induction) - ad.riposte_windup_can_parry_window < t)
		if not riposte_parry:
			if stage == CombatEnums.Stage.RECOVERY and ad.b_use_seamless_cftp_in_recovery and ai.b_can_combo:
				if sys.stamina < ai.feint_cost:
					return false
			else:
				# PostClash windup parry: Type PostClash, Windup, now < WindupEnd - LagInduction -
				# ClashOnParryCanParryWindow (+0xaac), ComingFromMotion IsA UBlockedMotion whose
				# FBlockResult.bClashOnParry (+0xb0) is set (decomp UAttackMotion.cpp 7518-7539)
				if type != CombatEnums.AttackType.POST_CLASH or stage != CombatEnums.Stage.WINDUP:
					return false
				if (windup_end - lag_induction) - ad.clash_on_parry_can_parry_window <= t:
					return false
				if not (coming_from is BlockedMotion) or not coming_from.b_clash_on_parry:
					return false
	elif stage == CombatEnums.Stage.RELEASE and not ad.b_can_block_from_release_after_hit:
		return false
	var b2 := modify_requested_block_type(bt)
	if not b_can_block:
		return false
	sys.assign_net_parry(b2)
	return true

# from UAttackMotion::OnFeinted rva=0x16312d0 (stamina part; the rest stops the montage)
func on_feinted(_lockout: float) -> void:
	var cost: int = ai.chamber_feint_cost if b_has_chambered else ai.feint_cost
	if cost != 0:
		sys.offset_stamina(-cost)

# from UAttackMotion::OnDynamicParamChanged_Implementation rva=0x1631060.
# bit 4: a miss-combo whose parent hit late loses MissComboExtraWindupIncrease; bit 1: hit -> recovery = HitRecovery;
# bit 2: chambered.
func on_dynamic_param_changed(_old: int, nv: int) -> void:
	var t := now()
	if (nv & CombatEnums.DYN_PARENT_HIT_LATE) != 0 and b_is_combo_from_miss:
		b_is_combo_from_miss = false
		if stage == CombatEnums.Stage.WINDUP:
			var x := ai.miss_combo_extra_windup_increase
			ai.windup -= x
			windup_end -= x
			end_time -= x
			release_end -= x
	if (nv & CombatEnums.DYN_HIT) != 0 and not b_has_hit:
		end_time = release_end
		first_hit_time = t
		# owning client (controller role 2): HitRecovery - LagInduction, clamped at 0 (OnDynamicParamChanged rva=0x1631060)
		var rec := ad.hit_recovery if sys.role != 2 else maxf(ad.hit_recovery - lag_induction, 0.0)
		end_time = rec + end_time
	b_has_chambered = (nv & CombatEnums.DYN_CHAMBERED) != 0
	b_has_hit = (nv & CombatEnums.DYN_HIT) != 0

# from UAttackMotion::OnEnded_Implementation rva=0x1631260: a move queued in recovery after a hit starts a Regular attack
func on_ended() -> void:
	if b_has_queued_move and b_has_hit:
		sys.assign_net_attack_motion(CombatEnums.AttackType.REGULAR, queued_move, queued_angle)
		if sys.motion != self:
			return
	super.on_ended()

# from UAttackMotion::CheckChamberIsValidIgnoreTiming rva=0x1617740 (this = the would-be chamberer, other = the
# attacking fighter): the chamberer has no bCannotChamber (+0xb8d); the other's motion is an attack; the game mode's
# CanChamber (AMordhauGameMode::CanChamber_Implementation rva=0x1586d60: !IsFriendly(source, target); no game mode ->
# allowed); this attack is Regular, or a Morph judged by the attack it morphed from (PreviousLastAttackMotion Move and
# AngleTarget); then both stabs -> valid; else a LeftStrike needs the other's RightStrike and a RightStrike the
# other's LeftStrike, with |AngleTarget - other AngleTarget| < ToChamberAttackAngleTolerance (+0xa80).
func chamber_valid_ignore_timing(other: MotionSystem) -> bool:
	if sys.character.b_cannot_chamber or other == null:
		return false
	var om = other.motion
	if not (om is AttackMotion):
		return false
	if sys.world.mode_rules != null and sys.world.is_friendly(sys, other):
		return false
	var m := move
	var ang := angle_target
	if type == CombatEnums.AttackType.MORPH and previous_last_attack != null:
		m = previous_last_attack.move
		ang = previous_last_attack.angle_target
	elif type != CombatEnums.AttackType.REGULAR and type != CombatEnums.AttackType.MORPH:
		return false
	if CombatEnums.is_stab(m) and CombatEnums.is_stab(om.move):
		return true
	if m == CombatEnums.Move.LEFT_STRIKE:
		if om.move != CombatEnums.Move.RIGHT_STRIKE:
			return false
	elif m == CombatEnums.Move.RIGHT_STRIKE:
		if om.move != CombatEnums.Move.LEFT_STRIKE:
			return false
	else:
		return false
	return absf(ang - om.angle_target) < ad.to_chamber_attack_angle_tolerance

# The non-geometry gates of UAttackMotion::CheckChamber rva=0x1617140 (decomp UAttackMotion.cpp 4730-5070):
# CheckChamberIsValidIgnoreTiming, then Stage Windup and now - StartTime < ChamberWindow (a Morph: StartTime and
# ChamberWindow of the attack it morphed from, +0x10d0), the validity again, and the masks: the other attack's Weapon
# AttackMask (+0x19a8) & this attack's Weapon ParryMask (+0x19ac) != 0. The geometry that follows (CheckSimpleBlock /
# CheckSimpleBlockDirectional angles, the BlockCollider forward test or BlockedAttacks memory) is the caller's
# contact (MeleeHit: a trace or scripted contact is geometry true).
func chamber_gate(other: MotionSystem) -> bool:
	if not chamber_valid_ignore_timing(other):
		return false
	var s := start_time
	var cw := ad.chamber_window
	if type == CombatEnums.AttackType.MORPH and previous_last_attack != null:
		s = previous_last_attack.start_time
		cw = previous_last_attack.ad.chamber_window
	if not (stage == CombatEnums.Stage.WINDUP and now() - s < cw):
		return false
	var ow: WeaponData = other.motion.weapon
	if ow == null or weapon == null:
		return false
	return (ow.attack_mask & weapon.parry_mask) != 0

func trace_fields() -> Dictionary:
	return {"type": type, "move": move, "stage": CombatEnums.STAGE_NAMES[stage], "windup_end": windup_end,
		"release_end": release_end, "end": end_time, "queued": b_has_queued_move, "hit": b_has_hit}

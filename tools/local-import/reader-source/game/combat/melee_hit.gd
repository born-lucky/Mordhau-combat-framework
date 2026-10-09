# melee_hit.gd - what a released attack does to the characters its weapon trace touches.
# Ports (extract/native/decomp/UAttackMotion.cpp unless noted):
#   ExecuteAttackTracingAndLogic rva=0x161c390: GlobalDamageModifier (+0xa84) = 1; weapon SampleTracers; for each
#     hit in HitResultCache order: ProcessHitForBlocking -> if it returns true, ProcessHitForDamage; stop at the first
#     hit that changes the attacker's motion or that ProcessHitForDamage consumes.
#   AMordhauGameState::LineTraceCharacters rva=0x159c4e0 -> AAdvancedCharacter::TraceSphericalLimbs rva=0x14a70f0
#     only traces SphericalLimbs, which no character Blueprint fills (BP_MordhauCharacter CDO has none); player hits
#     come from the world trace in AMordhauWeapon::SampleTracer rva=0x163be10 (UWorld line trace, channel 0xf) against
#     the character mesh's physics asset (UePhysics.body_boxes). UNCONFIRMED: the collision response of channel 0xf.
#   ProcessHitForBlocking rva=0x1638380: order and gates in process_hit_for_blocking (parry, then the victim's
#     attack: active parry, chamber, clash; else damage).
#   ProcessHitForDamage rva=0x1638a60 (see process_damage).
class_name MeleeHit

# Segment [a, b] against an oriented box: entry fraction in [0, 1], or -1 (slab test in box space).
static func segment_box(a: Vector3, b: Vector3, box_xf: Transform3D, half: Vector3) -> float:
	var inv := box_xf.affine_inverse()
	var p := inv * a
	var d := inv * b - p
	var t0 := 0.0
	var t1 := 1.0
	for k in 3:
		if absf(d[k]) < 1e-12:
			if absf(p[k]) > half[k]:
				return -1.0
			continue
		var ta := (-half[k] - p[k]) / d[k]
		var tb := (half[k] - p[k]) / d[k]
		if ta > tb:
			var s := ta; ta = tb; tb = s
		t0 = maxf(t0, ta)
		t1 = minf(t1, tb)
		if t0 > t1:
			return -1.0
	return t0

# All body boxes of `victim` crossed by segment [a, b]: [{t, bone}], nearest first (a line trace returns hits along
# the segment in distance order). Bones without a transform this tick are skipped.
static func trace_bodies(a: Vector3, b: Vector3, victim) -> Array:
	var hits := []
	for box in UePhysics.body_boxes():
		if not victim.bone_xf.has(box.bone):
			continue
		var t := segment_box(a, b, victim.bone_xf[box.bone] * (box.xf as Transform3D), box.half)
		if t >= 0.0:
			hits.append({"t": t, "bone": box.bone})
	hits.sort_custom(func(x, y): return x.t < y.t)
	return hits

# from UAttackMotion::ExecuteAttackTracingAndLogic rva=0x161c390 (character hits only). The HitResultCache it walks is
# filled by AMordhauWeapon::SampleTracers rva=0x163c430 -> AMordhauWeapon::SampleTracer rva=0x163be10, which this ports:
#  - one FCollisionQueryParams for all sample segments of the call (SampleTracers builds it once, seeded with the
#    weapon's ActorIgnoreCache: AddIgnoredActors(IgnoreActors)), so an actor added to it is skipped by later segments;
#  - per segment UWorld::LineTraceMultiByChannel(channel 0xf) -> SingleTraceHitsCache, read in its order (the engine
#    returns the hits by distance along the segment, across all actors; engine behaviour, UNCONFIRMED here);
#  - a hit on bone NAME_LeftHand (global 0x145723678) / NAME_RightHand (0x145723650) of a character whose Motion IsA
#    UParryMotion in Stage Parry (+0x53a == 0) is dropped (SampleTracer disasm 0x14163c20e..0x14163c25c);
#  - every other hit is appended to HitResultCache and, unless it is the character's BlockCollider or a weapon's clash
#    collider (never, for body boxes), its actor is added to the query's ignored actors.
static func trace_and_process(world, attacker) -> void:
	var atk = attacker.motion
	if not (atk is AttackMotion) or atk.stage != CombatEnums.Stage.RELEASE:
		return
	atk.global_damage_modifier = 1.0
	var cache := []					# weapon HitResultCache
	var segs: Array = attacker.tracer.segments()
	# after SampleTracers, Stage == Release: LastTraceStart/End = weapon CurrentTraceStart/End, bHasLastTrace = 1
	# (decomp UAttackMotion.cpp ExecuteAttackTracingAndLogic, `if (this->Stage == 1)` block)
	atk.last_trace_start = attacker.tracer.cur_start
	atk.last_trace_end = attacker.tracer.cur_end
	atk.b_has_last_trace = true
	var ignored: Array = attacker.tracer.actor_ignore_cache.duplicate()
	for seg in segs:
		sample_tracer(world, attacker, seg[0], seg[1], ignored, cache)
	for h in cache:
		var r: bool = process_hit(world, attacker, h.victim, h.bone)
		if attacker.motion != atk or r:
			return

# One AMordhauWeapon::SampleTracer rva=0x163be10 segment [a, b] (see trace_and_process): appends {victim, bone} to
# `cache` and the victims' ids to `ignored` (the shared FCollisionQueryParams ignored actors)
static func sample_tracer(world, attacker, a: Vector3, b: Vector3, ignored: Array, cache: Array) -> void:
	var single := []				# SingleTraceHitsCache of this segment, every actor, nearest first
	for nm in world.fighters:
		var v = world.fighters[nm]
		if v == attacker or v.dead or ignored.has(v.id):
			continue
		for h in trace_bodies(a, b, v):
			single.append({"t": h.t, "victim": v, "bone": h.bone})
	single.sort_custom(func(x, y): return x.t < y.t)
	for h in single:
		var v = h.victim
		if ignored.has(v.id):
			continue
		if (h.bone == "LeftHand" or h.bone == "RightHand") and v.motion is ParryMotion 				and v.motion.stage == CombatEnums.ParryStage.PARRY:
			continue
		cache.append({"victim": v, "bone": h.bone})
		ignored.append(v.id)

# ProcessHitForBlocking then ProcessHitForDamage for one hit. Returns true when tracing should stop.
static func process_hit(world, attacker, victim, bone: String) -> bool:
	var atk = attacker.motion
	if not (atk is AttackMotion):
		return true
	if atk.hit_actors.has(victim.id):			# weapon ActorSetCache (+0xe98): one hit per actor per release
		return false
	if not process_hit_for_blocking(world, attacker, victim, atk):
		return attacker.motion != atk
	return process_damage(world, attacker, victim, atk, bone)

# from UAttackMotion::ProcessHitForBlocking rva=0x1638380 (decomp UAttackMotion.cpp 4034-4403; character victims).
# true = go on to damage (ProcessHitForDamage). The trace here only reaches body boxes, so the hit component is never
# the victim's BlockCollider, a weapon's clash collider or a shield box (the exe's bVar5 / cVar12 / cVar13 = false).
# Order, as the exe runs it:
#  1. the victim's LastParryMotion (+0xf0, current or not) when the attack bCanBeParriedInEarlyRelease (+0xa6c) or is
#     not in early release (IsInEarlyRelease, vcall +0x350): UParryMotion::CheckParry -> HandleWasParried; then chip
#     damage unless the attack is in live recovery (IsInLiveRecovery, +0x358); stop.
#  2. the victim's current motion is a UAttackMotion and this attack is not in live recovery:
#     a. CheckAttackParry (vcall +0x338, the riposte "active parry", rva=0x1616cd0): succeeds only for a hit on the
#        defender's BlockCollider or shield box, or an attack already in its BlockedAttacks memory (filled by such a
#        hit); body-box hits never qualify, so it is false here (not ported, with the BlockCollider trace).
#     b. CheckChamber (vcall +0x330, rva=0x1617140; AttackMotion.chamber_gate + check_chamber) -> chip damage; stop.
#     c. a kick or fists attack (Move 4, Weapon IsA AFistsWeapon) on the body: CheckClash (vcall +0x340,
#        rva=0x16179a0; not ported: needs the victim's weapon ClashCollider geometry) -> otherwise damage.
#  3. otherwise damage.
static func process_hit_for_blocking(world, attacker, victim, atk) -> bool:
	var live_recovery: bool = atk.is_in_live_recovery()
	var pm = victim.last_parry_motion
	if pm != null and (atk.ad.b_can_be_parried_in_early_release or not atk.is_in_early_release()) 			and pm.check_parry(attacker):
		# CheckParry's success tail: bDrainAllStamOnBlock costs the defender 100 (0xffffff9c at 0x14164f2cf), then
		# ReceiveBlock with float(int(StaminaDrain)) (+ ExtraStaminaDrainVsHeldBlock when holdable): disasm
		# 0x14164f2a3..0x14164f308; AMordhauCharacter::OnBlockedMelee (event)
		var drain := float(int(atk.ai.stamina_drain))
		if pm.b_is_block_holdable:
			drain += atk.ai.extra_stamina_drain_vs_held_block
		if atk.ai.b_drain_all_stam_on_block:
			victim.offset_stamina(-100)
		pm.receive_block(drain, atk.move, attacker)
		world.trace_event("%s parried %s" % [victim.name, attacker.name])
		world.emit_event({"kind": "parry", "attacker": attacker.name, "victim": victim.name, "move": atk.move})
		handle_was_parried(attacker, victim, atk, pm)
		if not live_recovery:
			chip_damage(attacker, victim, atk)
		return false
	var dm = victim.motion
	if not (dm is AttackMotion) or live_recovery:
		return true
	if dm.chamber_gate(attacker):
		check_chamber(world, attacker, victim, atk, dm)
		chip_damage(attacker, victim, atk)
		return false
	return true

# Chip damage of ProcessHitForBlocking (decomp UAttackMotion.cpp 4245-4275), after a parry or a chamber: when the
# attack's ChipDamagePercentageOnBlock > 0 (only BP_Polehammer_Horde sets one), ComputeMeleeDamage (vcall +0x948)
# on bone NAME_Spine1 (global 0x145723618, labels_data.tsv "NAME_Spine1") x the percentage, through TakeDamage
# (vcall +0x590) with an FMordhauDamageEvent (Melee) on that bone.
static func chip_damage(attacker, victim, atk) -> void:
	if atk.ai.chip_damage_percentage_on_block > 0.0:
		victim.take_damage(MeleeDamage.compute(victim, atk.ai, "Spine1") * atk.ai.chip_damage_percentage_on_block, attacker)

# The stamina-break branch of UAttackMotion::CheckChamber rva=0x1617140: the chamberer's stamina reached 0 paying a
# ChamberCost > 0. stun = the attacker HasPerk(0x15) (perks are not ported: false); then forced stun without disarm when
# the chamberer's attack Weapon has !bAllowDrop (+0xcb8), the chamberer bDestroyEquipmentOnDeath (+0x11e0) or the
# attacker bAlwaysStunInsteadOfDisarm (+0x1219). direction = -90 when the attacker's Move is LeftStrike, else 90.
# Chamberer: stamina += its attack's ChamberStaminaRecover (+0xa7c), then Disarmed(direction) or
# Stunned(direction, bone 0, disarm); attacker: stamina += ChamberStaminaRecover, Blocked(Clash) carrying the same
# bIsStun / bIsDisarm bits (FBlockResult byte 1 / 2 -> Param1 bits 1 / 2), time 0. No Chambered dynamic bit.
static func _chamber_stamina_break(world, attacker, victim, atk, dm) -> void:
	var stun := false
	var disarm := false
	var direction := -90.0 if atk.move == CombatEnums.Move.LEFT_STRIKE else 90.0
	var dm_drop: bool = dm.weapon != null and dm.weapon.b_allow_drop
	if not dm_drop or victim.character.b_destroy_equipment_on_death or attacker.character.b_always_stun_instead_of_disarm:
		stun = true
		disarm = false
	victim.offset_stamina(dm.ad.chamber_stamina_recover)
	if not stun:
		disarm = true
		victim.assign_net_disarmed(direction)
	else:
		victim.assign_net_stunned(direction, 0, disarm)
	attacker.offset_stamina(dm.ad.chamber_stamina_recover)
	attacker.assign_net_blocked(CombatEnums.BlockedReason.CLASH, int(stun) | (2 if disarm else 0), 0.0)
	world.trace_event("%s chamber stamina break: %s" % [victim.name, "stun" if stun else "disarm"])
	world.emit_event({"kind": "stamina_break", "who": victim.name, "by": attacker.name, "stun": stun, "disarm": disarm})

# from UAttackMotion::HandleWasParried rva=0x162b4d0: when the parrier is now in a UDisarmedMotion, or a UStunMotion
# (bIsStun; its bWillDisarm +0xa0 also counts as disarm), the attacker gets Blocked(Clash, Param1 = stun | disarm x 2)
# (AssignNetMotion {0x30600}: MotionType 6, Param0 3) and stamina += the parry's BlockStaminaRecover (+0x518); else
# UAttackMotion::PutUsInBlockedMotionFromParry rva=0x163a7d0: if bWillClashWhenParried -> Blocked(Clash,
# bClashOnParry), else Blocked(Parry, time = GetFastestAttackWindup(parrier) rva=0x1622f40)
static func handle_was_parried(attacker, parrier, atk, pm = null) -> void:
	var pmot = parrier.motion
	var disarmed: bool = pmot is DisarmedMotion
	var stunned: bool = pmot is StunMotion
	if stunned and pmot.b_will_disarm:
		disarmed = true
	if disarmed or stunned:
		attacker.assign_net_blocked(CombatEnums.BlockedReason.CLASH, int(stunned) | (2 if disarmed else 0), 0.0)
		if pm != null:
			attacker.offset_stamina(pm.pd.block_stamina_recover)
		return
	if atk.ai.b_will_clash_when_parried:
		# FBlockResult {reason 3, byte 7 = 1} (CheckChamber 0x141617649..0x141617671) -> Param1 bit 0x40:
		attacker.assign_net_blocked(CombatEnums.BlockedReason.CLASH, 64, 0.0)	# FNetMotion::Blocked rva=0x1615f50 (byte 7 -> or al, 0x40 at 0x141615fe1)
		return
	attacker.assign_net_blocked(CombatEnums.BlockedReason.PARRY, 0, fastest_attack_windup(parrier))

# from UAttackMotion::GetFastestAttackWindup rva=0x1622f40: min(StabAttack.Windup x RiposteWindupModifier of the
# Stab motion class, StrikeAttack.Windup x RiposteWindupModifier of the RightStrike motion class); 0.4
# (CombatConstants.no_weapon_fastest_windup) without a weapon. (Vehicle / alt-mode profile selection not ported.)
static func fastest_attack_windup(c) -> float:
	if c.weapon == null:
		return CombatConstants.no_weapon_fastest_windup
	var stab: float = c.weapon.stab.windup
	var strike: float = c.weapon.strike.windup
	stab *= c.attack_motion_defaults(CombatEnums.Move.STAB).riposte_windup_modifier
	strike *= c.attack_motion_defaults(CombatEnums.Move.RIGHT_STRIKE).riposte_windup_modifier
	return minf(strike, stab)

# from UAttackMotion::CheckChamber rva=0x1617140 after its gates (AttackMotion.chamber_gate; geometry = the contact):
# the attacker's bDrainAllStamOnBlock (AttackInfo +0x99) costs the chamberer 100; the chamberer pays ChamberCost
# unless already chambered; it gets dynamic bit
# Chambered; the chambered attacker goes to Blocked(Clash, bClashOnParry) if bWillClashWhenParried, else
# Blocked(Chamber, time = chamberer WindupEnd - now) (for a Morph, the morphed-from attack's WindupEnd).
# (bRagdollOnBlock -> the chamberer's ragdoll vcall +0x8f0: no ragdoll here.)
# Stamina break (decomp UAttackMotion.cpp CheckChamber, `GetStatValue == 0 && 0 < cost`): see _chamber_stamina_break.
static func check_chamber(world, attacker, victim, atk, dm) -> void:
	if atk.ai.b_drain_all_stam_on_block:
		victim.offset_stamina(-100)
	var cost: int = 0 if dm.b_has_chambered else dm.ai.chamber_cost
	victim.offset_stamina(-cost)
	if victim.stamina == 0 and 0 < cost:
		_chamber_stamina_break(world, attacker, victim, atk, dm)
		return
	victim.assign_net_motion_dynamic_param(victim.net.dynamic_param | CombatEnums.DYN_CHAMBERED)
	var we: float = dm.windup_end
	if dm.type == CombatEnums.AttackType.MORPH and dm.previous_last_attack != null:
		we = dm.previous_last_attack.windup_end
	if atk.ai.b_will_clash_when_parried:
		# FBlockResult {reason 3, byte 7 = 1} (CheckChamber 0x141617649..0x141617671) -> Param1 bit 0x40:
		attacker.assign_net_blocked(CombatEnums.BlockedReason.CLASH, 64, 0.0)	# FNetMotion::Blocked rva=0x1615f50 (byte 7 -> or al, 0x40 at 0x141615fe1)
	else:
		attacker.assign_net_blocked(CombatEnums.BlockedReason.CHAMBER, 0, we - world.now)
	world.trace_event("%s chambered %s" % [victim.name, attacker.name])
	world.emit_event({"kind": "chamber", "attacker": attacker.name, "victim": victim.name})

# from UAttackMotion::ProcessHitForDamage rva=0x1638a60 (character victim, no vehicles/perks/teams/glance):
#  - a strike after LastReleaseNormalizedTime >= 1 - StrikeAnimationNormalizedRecoveryOffset does nothing
#  - damage = AMordhauCharacter::ComputeMeleeDamage(Damage, HeadBonus, LegBonus, bone) (victim vtable +0x948)
#      x GlobalDamageModifier (+0xa84) x victim's RiposteTradeDamageFactor (+0xa9c) when the victim is in an
#      unfinished Riposte that did not come from a shield-wall parry
#  - victim stamina -= StaminaDamage (AttackInfo +0x5c); victim TakeDamage (vtable +0x590)
#  - victim flinches (FNetMotion::Flinched, FlinchDurationModifier, FlinchSpeedModifier) when its motion is flinchable
#  - attacker stamina += HitStaminaReward (+0xa8) + ExtraStaminaOnHit (character +0xe40)
#  - friendly = AMordhauGameState::IsFriendly(owner, victim) (disasm 0x141639263, kept in [rbp-0x60]); an attack that
#    already hit a friend (bHasHitFriendly +0x108c) scales later hits by PostFriendlyHitModifier (+0xa8c), then
#    bHasHitFriendly |= friendly (ProcessHitForDamage 0x1416392ae..0x1416392c5)
#  - a friendly hit flinches only when the game mode's TeamDamageFlinch (+0x414) != 0; it gives no HitStaminaReward
#    (`if (!bIsFriendly && reward != 0) OffsetStamina`)
#  - UAttackMotion::CanContinueTracingAfterDealingDamage rva=0x16164f0 (ProcessHitForDamage vcall +0x398 at 0x141639b94; dl =
#    victim, r8b = friendly, r9b = victim bIsDead after the damage, 4th = riposte trade, unused by the body):
#    continue when (bHasKilled and !bStopOnHitOnKills +0xa90) or (!(friendly and HitStopOnTeam) and the victim's
#    bWillStopMelee (+0x9b1) / damageable bStopsMeleeAttacks (+0xe9) are clear and !AttackInfo.bStopOnHit (+0xed0));
#    HitStopOnTeam = !(owner AMordhauCharacter bIsHitStopOnTeamHitsDisabled +0xe64) and !(game mode +0x390). Else
#    AssignNetMotion {MotionType 6 Blocked, Param0 4 Hit} (0x40600). Continuing sets the dynamic param bit Hit.
# Returns true (stop tracing).
static func process_damage(world, attacker, victim, atk, bone: String) -> bool:
	if CombatEnums.is_strike(atk.move) and \
			1.0 - atk.ad.strike_animation_normalized_recovery_offset <= atk.last_release_norm:
		return true
	atk.hit_actors.append(victim.id)
	attacker.tracer.actor_ignore_cache.append(victim.id)
	var factor: float = atk.global_damage_modifier
	var riposte_trade := false
	var vm = victim.motion
	var victim_flinchable: bool = vm != null and vm.b_is_flinchable
	if vm is AttackMotion and vm.type == CombatEnums.AttackType.RIPOSTE and vm.stage != CombatEnums.Stage.RECOVERY:
		var from_wall: bool = vm.coming_from is ParryMotion and vm.coming_from.b_is_shield_wall
		if not from_wall:
			factor *= vm.ad.riposte_trade_damage_factor
			riposte_trade = true
	var friendly: bool = world.is_friendly(attacker, victim)
	if atk.b_has_hit_friendly:
		factor *= atk.ad.post_friendly_hit_modifier
	atk.b_has_hit_friendly = atk.b_has_hit_friendly or friendly
	var dmg: float = MeleeDamage.compute(victim, atk.ai, bone) * factor
	if atk.ai.stamina_damage != 0.0:
		victim.offset_stamina(-int(atk.ai.stamina_damage))
	var applied: float = victim.take_damage(dmg, attacker)
	world.trace_event("%s hit %s %s %.2f" % [attacker.name, victim.name, bone, dmg])
	world.emit_event({"kind": "hit", "attacker": attacker.name, "victim": victim.name, "bone": bone, "damage": dmg,
		"applied": applied, "health": victim.health, "friendly": friendly})
	var team_flinch: bool = world.mode_rules != null and world.mode_rules.team_damage_flinch != 0
	if victim_flinchable and not victim.dead and (not friendly or team_flinch):
		victim.assign_net_flinched(0.0, false, atk.ai.flinch_duration_modifier, atk.ai.flinch_speed_modifier)
	var reward: float = atk.ai.hit_stamina_reward + float(attacker.character.extra_stamina_on_hit)
	if reward != 0.0 and not friendly:
		attacker.offset_stamina(int(reward))
	var killed: bool = victim.dead
	var stop_on_team: bool = not attacker.character.b_is_hit_stop_on_team_hits_disabled \
		and not (world.mode_rules != null and world.mode_rules.b_is_hit_stop_on_team_hits_disabled)
	var cont: bool = (killed and not atk.ad.b_stop_on_hit_on_kills) or (not (friendly and stop_on_team) \
		and not victim.character.b_will_stop_melee and not atk.ai.b_stop_on_hit)
	var stop: bool = not cont
	if stop:
		attacker.assign_net_blocked(CombatEnums.BlockedReason.HIT, 0, 0.0)
		return true
	attacker.assign_net_motion_dynamic_param(attacker.net.dynamic_param | CombatEnums.DYN_HIT)
	return true

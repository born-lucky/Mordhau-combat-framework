# combat_constants.gd - every literal the combat rules (game/combat/*) take from the exe's .rdata, loaded once and
# given a name, so gameplay code reads `CombatConstants.easy_parry_window` instead of an address (the OpenMW split:
# mechanics ask a store for "fCombatBlockLeftAngle", only the loader knows where it lives).
# Source: extract/native/rdata.tsv via UeRdata (raw IEEE-754 bytes at the va; scripts/rdata_consts.py, hash-checked exe).
# Each value is declared with `_k(name, va, functions)`: `functions` are the game functions whose code loads that va
# (Ghidra `_DAT_<va>`); test_combat checks every one of them is listed in the rdata.tsv `funcs` column for that va and
# that the value equals the raw bytes. One exe literal can back several names (the compiler pools equal floats), so
# names follow the rule that uses the value, not the value.
# FNames: the bone names the damage rules compare against are module-static FName globals built by dynamic
# initializers; rdata.tsv names each source string's initializer ("dynamic initializer for 'NAME_<x>'"), see fname().
class_name CombatConstants

static var cites := {}		# name -> [va, PackedStringArray of citing functions, value]

static func _k(name: String, va: int, functions: Array) -> float:
	var v := UeRdata.f32(va)
	cites[name] = [va, PackedStringArray(functions), v]
	return v

# ---- numeric helpers -------------------------------------------------------------------------------------------
# UE SMALL_NUMBER (1e-8): "is this span degenerate" in the range maps / divisions below
static var small_number: float = _k("small_number", 0x144014a88, ["UAttackMotion::GetSmoothedWindUpNormalizedTime",
	"UFeintedMotion::OnBegin_Implementation", "UMordhauUtilityLibrary::PackFloat",
	"UHealthStatComponent::SetStatValue_Internal", "UBlockedMotion::OnBegin_Implementation",
	"UBlockedMotion::OnTick_Implementation"])
# 1/255: a replicated byte (FNetMotion Param / DynamicParam) back to [0, 1]
static var byte_to_unit: float = _k("byte_to_unit", 0x1440a4394, ["UAttackMotion::OnBegin_Implementation",
	"UFlinchMotion::OnBegin_Implementation"])
# 255: [0, 1] -> byte (PackFloat) and the byte clamp of FNetMotion::Blocked's time
static var byte_max: float = _k("byte_max", 0x144014b78, ["UMordhauUtilityLibrary::PackFloat", "FNetMotion::Blocked"])

# ---- UAttackMotion / UStrikeMotion -----------------------------------------------------------------------------
# 0.002 s: one lag unit of FNetMotion Param2 (OnBegin subtracts Param2 x 0.002 from the windup)
static var net_lag_unit_s: float = _k("net_lag_unit_s", 0x1440e9f84, ["UAttackMotion::OnBegin_Implementation"])
# 0.5 s: EasyParryUntilTime = now + 0.5 during a riposte / chambered windup (OnTick) and at a flinch (OnBegin)
static var easy_parry_window: float = _k("easy_parry_window", 0x143fe4e04, ["UAttackMotion::OnTick_Implementation",
	"UFlinchMotion::OnBegin_Implementation"])
# 60: a strike morphing into a stab keeps AngleTarget x 60 as the requested angle
static var morph_stab_angle_scale: float = _k("morph_stab_angle_scale", 0x143fe4e38,
	["UStrikeMotion::ModifyRequestedMorphAttack"])
# 0.4 s: GetFastestAttackWindup without a weapon
static var no_weapon_fastest_windup: float = _k("no_weapon_fastest_windup", 0x1441231f0,
	["UAttackMotion::GetFastestAttackWindup"])

# ---- AutoBlend (UAttackMotion::OnBegin_Implementation search, AutoBlendCalculateWeaponDistance) -------------------
static var auto_blend_deg_to_rad: float = _k("auto_blend_deg_to_rad", 0x144022360,
	["UAttackMotion::AutoBlendCalculateWeaponDistance"])
# 15: radians x weapon Length -> distance units
static var auto_blend_angle_to_cm: float = _k("auto_blend_angle_to_cm", 0x14402cb98,
	["UAttackMotion::AutoBlendCalculateWeaponDistance"])
# 0.15 s: a blend below it needs no second sample
static var auto_blend_min_blend: float = _k("auto_blend_min_blend", 0x1442efa58, ["UAttackMotion::OnBegin_Implementation"])
# 0.5: averaging the two samples, and the latest second-sample time
static var auto_blend_half: float = _k("auto_blend_half", 0x143fe4e04, ["UAttackMotion::OnBegin_Implementation"])
# FLT_MAX: best blend before the first step
static var auto_blend_no_blend_yet: float = _k("auto_blend_no_blend_yet", 0x144014b94,
	["UAttackMotion::OnBegin_Implementation"])

# ---- UBlockedMotion --------------------------------------------------------------------------------------------
# 0.025 s: one unit of the Blocked Param2 time byte
static var blocked_time_unit_s: float = _k("blocked_time_unit_s", 0x14402c9a0, ["UBlockedMotion::OnBegin_Implementation"])
# NextKickTime = EndTime + 0.3 after a Chamber / Clash, + 0.2 after a Parry (World / Hit leave it: `(Reason - World) & 0xfd`)
static var blocked_kick_delay_chamber: float = _k("blocked_kick_delay_chamber", 0x1441231e4,
	["UBlockedMotion::OnBegin_Implementation"])
static var blocked_kick_delay_parry: float = _k("blocked_kick_delay_parry", 0x1442898a4, ["UBlockedMotion::OnBegin_Implementation"])
# 0.4 s: a blocked kick gets its movement back (OriginalMovementRestriction) this long after the Blocked motion began,
# and the procedural release bounce runs only before StartTime + 0.4 (UBlockedMotion::OnTick_Implementation)
static var blocked_bounce_window: float = _k("blocked_bounce_window", 0x1441231f0, ["UBlockedMotion::OnTick_Implementation"])
# 0.75: LastReleaseNormalizedTime above it picks the attack's ParryLateBounceCurve instead of ParryBounceCurve (OnTick)
static var blocked_late_bounce_release: float = _k("blocked_late_bounce_release", 0x143fe4e08,
	["UBlockedMotion::OnTick_Implementation"])
# 0.5: bounce position = bounce x scales + LastReleaseNormalizedTime x 0.5 + 0.5 (OnTick, SetAnimPosition argument)
static var blocked_bounce_half: float = _k("blocked_bounce_half", 0x143fe4e04, ["UBlockedMotion::OnTick_Implementation"])
# 1.0: the release scale when the motion has no release-scale curve (OnTick, before UCurveFloat::GetFloatValue)
static var blocked_unit_scale: float = _k("blocked_unit_scale", 0x143fe4e0c, ["UBlockedMotion::OnTick_Implementation"])
# -1: "no blend-out" value of KickHitStopBlendOutTime (OnBegin skips StopAnim when it equals -1)
static var blocked_no_blend_out: float = _k("blocked_no_blend_out", 0x144014bb8, ["UBlockedMotion::OnBegin_Implementation"])
# 0.5 s: OnLeave stops the bounce montage with this blend when the procedural bounce had not faded out yet
static var blocked_leave_fade: float = _k("blocked_leave_fade", 0x143fe4e04, ["UBlockedMotion::OnLeave_Implementation"])

# ---- UIdleMotion / UFlinchMotion -------------------------------------------------------------------------------
# 0.5 s: Idle forgets the motion it came from (ComingFrom) this long after it began
static var idle_coming_from_timeout: float = _k("idle_coming_from_timeout", 0x143fe4e04, ["UIdleMotion::OnTick_Implementation"])
# 0.25 s: NextKickTime = flinch EndTime + 0.25
static var flinch_kick_delay: float = _k("flinch_kick_delay", 0x143fe4e00, ["UFlinchMotion::OnBegin_Implementation"])
# 0.3 s: NextKickTime = disarmed EndTime + 0.3 (the same .rdata float as blocked_kick_delay_chamber)
static var disarmed_kick_delay: float = _k("disarmed_kick_delay", 0x1441231e4, ["UDisarmedMotion::OnBegin_Implementation"])
# 0.45 s: a flinch gives back atmospherics / offhand IK and plays its armour foley this long after it began
static var flinch_recover_cosmetics: float = _k("flinch_recover_cosmetics", 0x1443145ac, ["UFlinchMotion::OnTick_Implementation"])

# ---- UParryMotion ----------------------------------------------------------------------------------------------
# 1e6 s: ParryEnd while the parry is open (closed by OnDynamicParamChanged)
static var parry_open_end: float = _k("parry_open_end", 0x144349db0, ["UParryMotion::OnBegin_Implementation"])
# 0.25 s: shield-wall riposte window / queued-riposte grace
static var shield_wall_riposte_window: float = _k("shield_wall_riposte_window", 0x143fe4e00,
	["UParryMotion::OnDynamicParamChanged_Implementation", "UParryMotion::ProcessAttack_Implementation"])
# -0.5: the round-half bias of the shield-wall drain's RoundToInt (rint(-0.5 - 2x) >> 1)
static var shield_wall_drain_round_bias: float = _k("shield_wall_drain_round_bias", 0x144022404,
	["UParryMotion::ReceiveBlock"])

# ---- UMotionSystemComponent / FNetMotion -----------------------------------------------------------------------
# 0.3 s: a kick requested within 0.3 s of NextKickTime carries the remaining wait as lag
static var kick_lag_window: float = _k("kick_lag_window", 0x1441231e4, ["UMotionSystemComponent::AssignNetAttackMotion"])
# 0.3 s: every kick's Windup + 0.3 (UKickMotion::ModifyAttackInfo_Implementation disasm 0x14165d773 addss); cancels
# the 0.3 s lag a kick sent with NextKickTime passed carries (kick_lag_window)
static var kick_windup_extra: float = _k("kick_windup_extra", 0x1441231e4, ["UKickMotion::ModifyAttackInfo_Implementation"])
# 0.08 s: ping compensation clamp
static var max_ping_compensation: float = _k("max_ping_compensation", 0x144331154,
	["UMotionSystemComponent::AssignNetAttackMotion"])
# 500: seconds -> lag units (inverse of net_lag_unit_s)
static var net_lag_units_per_s: float = _k("net_lag_units_per_s", 0x1442efaa4, ["UMotionSystemComponent::AssignNetAttackMotion"])
# attack angle clamp [-60, 60] degrees, then x 1/60 to [-1, 1] before PackFloat
static var attack_angle_min: float = _k("attack_angle_min", 0x1443247d4, ["UMotionSystemComponent::AssignNetAttackMotion"])
static var attack_angle_max: float = _k("attack_angle_max", 0x143fe4e38, ["UMotionSystemComponent::AssignNetAttackMotion"])
static var attack_angle_to_unit: float = _k("attack_angle_to_unit", 0x14428988c,
	["UMotionSystemComponent::AssignNetAttackMotion"])
# 40: Blocked time seconds -> byte (x 40, the inverse of blocked_time_unit_s)
static var blocked_time_to_byte: float = _k("blocked_time_to_byte", 0x144331164,
	["FNetMotion::Blocked", "UAttackMotion::PutUsInBlockedMotionFromParry"])
# 1/180: flinch angle degrees -> [-1, 1] before PackFloat
static var flinch_angle_to_unit: float = _k("flinch_angle_to_unit", 0x144014a90, ["FNetMotion::Flinched"])

# ---- damage ----------------------------------------------------------------------------------------------------
# 1: a positive hit does at least 1 damage
static var min_damage: float = _k("min_damage", 0x143fe4e0c, ["UDamageableComponent::ModifyDamage"])

# ---- spawn / friendly damage (AMordhauCharacter::TakeDamage rva=0x156e980, UDamageableComponent::ModifyDamage) --------
# 0.02: damage scale for a friendly hit within friendly_spawn_window of the victim's spawn (TakeDamage), and for any
# hit inside the damageable component's SpawnProtectionDuration (ModifyDamage)
static var spawn_damage_scale: float = _k("spawn_damage_scale", 0x14432475c,
	["AMordhauCharacter::TakeDamage", "UDamageableComponent::ModifyDamage"])
# 5 s: friendly hits on a character younger than this are scaled by spawn_damage_scale
static var friendly_spawn_window: float = _k("friendly_spawn_window", 0x143fe4e20, ["AMordhauCharacter::TakeDamage"])
# 5000: damage at or above this ignores the game mode's SpawnProtectionDuration
static var spawn_protection_max_damage: float = _k("spawn_protection_max_damage", 0x144349dac,
	["AMordhauCharacter::TakeDamage"])

# ---- AMordhauWeapon tracers ------------------------------------------------------------------------------------
# 1/15: tracer span (cm) -> weapon Length
static var tracer_length_per_cm: float = _k("tracer_length_per_cm", 0x1440701e0, ["AMordhauWeapon::RecalculateTracerPoints"])
# 2/7.5 and 0.5: sample count = rint(length_cm x 2/7.5 + 0.5) >> 1
static var tracer_count_scale: float = _k("tracer_count_scale", 0x144362ca8, ["AMordhauWeapon::SampleTracers"])
static var tracer_round_bias: float = _k("tracer_round_bias", 0x143fe4e04, ["AMordhauWeapon::SampleTracers"])
# 7.5 cm between samples, walked from the far end with step -1
static var tracer_spacing_cm: float = _k("tracer_spacing_cm", 0x144362cc0, ["AMordhauWeapon::SampleTracers"])
static var tracer_step: float = _k("tracer_step", 0x144014bb8, ["AMordhauWeapon::SampleTracers"])

# ---- bone FNames (AMordhauCharacter::IsHead rva=0x154aa40 / IsLeftLeg rva=0x154ad60 / IsRightLeg rva=0x154b880) ---
# global -> string pairing: disassembly of the initializers at 0x14063f0f0.. ("lea rdx, str; lea rcx, global;
# jmp FName::FName"). IsHead compares _DAT_145721940 = NAME_Head; IsRightLeg _DAT_1457219c8/c0/d0; IsLeftLeg
# _DAT_1457219e0/d8/e8.
static var fname_cites := {}	# string va -> FName text

# "`dynamic initializer for 'NAME_RightUpLeg''" (rdata.tsv funcs of the string's va) -> "RightUpLeg"
static func fname(va: int) -> String:
	if fname_cites.has(va):
		return fname_cites[va]
	var n := ""
	for f in UeRdata.funcs(va):
		var i := f.find("'NAME_")
		if i >= 0:
			n = f.substr(i + 6).get_slice("'", 0)
	fname_cites[va] = n
	return n

static func _fnames(vas: Array) -> PackedStringArray:
	var out := PackedStringArray()
	for va in vas:
		out.append(fname(va))
	return out

static var head_bones: PackedStringArray = _fnames([0x144317d48])							# NAME_Head
static var right_leg_bones: PackedStringArray = _fnames([0x144317ba0, 0x144317b90, 0x144317bb0])	# RightLeg/UpLeg/Foot
static var left_leg_bones: PackedStringArray = _fnames([0x144317bd0, 0x144317bc0, 0x144317bd8])	# LeftLeg/UpLeg/Foot

# ---- armour coverage FNames (AMordhauCharacter::BuildCharacter rva=0x15319e0, game/character/armor.gd) -----------
# WearableProtectionCoverageMap keys in BuildCharacter's Emplace order; global -> string pairing as above (armor.gd
# header lists each _DAT_1457219xx global with its string va).
static var armor_head_bones: PackedStringArray = _fnames([0x144317d48, 0x144317ac8])		# head, Neck
static var armor_upper_chest_bones: PackedStringArray = _fnames([0x144317aec, 0x144317ae0, 0x144317ad8, 0x144317ad0,
	0x144317b70, 0x144317b60, 0x144317b58, 0x144317b28, 0x144317b18, 0x144317b08])
static var armor_leg_bones: PackedStringArray = _fnames([0x144317bc0, 0x144317bd0, 0x144317bd8, 0x144317b90,
	0x144317ba0, 0x144317bb0])
# FootstepArmorTier = max(tier(Spine1), tier(RightLeg))
static var footstep_spine1_bone: String = fname(0x144317ad0)
static var footstep_right_leg_bone: String = fname(0x144317ba0)
# UKickMotion::ModifyAttackInfo_Implementation tests GetArmorTierForBone(NAME_RightLeg) (FName global 0x145723d60)
static var kick_tier_bone: String = fname(0x144317ba0)

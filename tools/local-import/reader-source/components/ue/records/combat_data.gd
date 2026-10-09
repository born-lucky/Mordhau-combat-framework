# combat_data.gd - every constant the combat port reads, from the user's extract/ only.
# class_defaults(bp) = native constructor defaults (NativeCtor, extract/native/decomp + types) with the Blueprint CDO
# chain (UePkg.defaults, extract/json) merged over them, child wins, struct fields merged one by one. This is the
# value UE gives the object at spawn: the CDO is constructed natively, then each Blueprint's serialized deltas apply.
class_name CombatData

const CHARACTER := "Mordhau/Content/Mordhau/Blueprints/Characters/BP_MordhauCharacter"
const MOTIONS := "Mordhau/Content/Mordhau/Blueprints/Motions/"
const FEINTED_MOTION := MOTIONS + "BP_FeintedMotion"
const BLOCKED_MOTION := MOTIONS + "BP_BlockedMotion"
const FLINCH_MOTION := MOTIONS + "BP_FlinchMotion"
const STUN_MOTION := MOTIONS + "BP_StunMotion"			# BP_MordhauCharacter CDO Motions[4]
const DISARMED_MOTION := MOTIONS + "BP_DisarmedMotion"	# BP_MordhauCharacter CDO Motions[7]

static var _cache := {}

# "StrikeMotion" (UePkg.native_root) -> "UStrikeMotion"; actors use the A prefix ("MordhauWeapon" -> AMordhauWeapon)
static func native_class(root_name: String) -> String:
	for p in ["U", "A", "F"]:
		if NativeCtor.has_type(p + root_name):
			return p + root_name
	return ""

# IsA(AMordhauWeapon) for a native class name, walking the PDB base chain (NativeCtor.layout(cls).base,
# extract/native/types/<cls>.h: AMordhauShield : AMordhauWeapon : AMordhauEquipment ...)
static func is_weapon_class(cls: String) -> bool:
	return is_class_of(cls, "AMordhauWeapon")

static func is_class_of(cls: String, base: String) -> bool:
	var cur := cls
	for _i in 32:
		if cur == base:
			return true
		if cur == "":
			return false
		cur = String(NativeCtor.layout(cur).base)
	return false

# The game mode / game state settings the damage chain reads (CombatState.set_game_mode). Names = the PDB fields
# (types/AMordhauGameMode.h, AMordhauGameState.h); values = the merged native ctor + Blueprint CDO chain.
class ModeRules:
	var game_mode := ""
	var game_state := ""
	var damage_factor := 1.0					# AMordhauGameMode +0x40c DamageFactor
	var team_damage_factor := 1.0				# +0x410 TeamDamageFactor (ctor 0.25)
	var team_damage_flinch := 0					# +0x414 TeamDamageFlinch
	var spawn_protection_duration := 0.0		# +0x418 SpawnProtectionDuration (ctor 0.2)
	var b_disable_damage := false				# +0x41c bDisableDamage
	var b_is_hit_stop_on_team_hits_disabled := false	# +0x390
	var b_is_team_mode := false					# AMordhauGameState +0x6b0 bIsTeamMode
	var damageable_spawn_protection_duration := 0.0	# UDamageableComponent +0xe4 SpawnProtectionDuration (ctor 0.2)

static func mode_rules(game_mode_bp: String, game_state_bp: String) -> ModeRules:
	var m := ModeRules.new()
	var gm := class_defaults(game_mode_bp)
	var gs := class_defaults(game_state_bp)
	m.game_mode = game_mode_bp
	m.game_state = game_state_bp
	m.damage_factor = float(gm.get("DamageFactor", 1.0))
	m.team_damage_factor = float(gm.get("TeamDamageFactor", 1.0))
	m.team_damage_flinch = int(gm.get("TeamDamageFlinch", 0))
	m.spawn_protection_duration = float(gm.get("SpawnProtectionDuration", 0.0))
	m.b_disable_damage = bool(gm.get("bDisableDamage", false))
	m.b_is_hit_stop_on_team_hits_disabled = bool(gm.get("bIsHitStopOnTeamHitsDisabled", false))
	m.b_is_team_mode = bool(gs.get("bIsTeamMode", false))
	m.damageable_spawn_protection_duration = float(native_only("UDamageableComponent").get("SpawnProtectionDuration", 0.0))
	return m

# Native-only class (no Blueprint): its constructor defaults, tagged like class_defaults
static func native_only(cls: String) -> Dictionary:
	var key := "native:" + cls
	if not _cache.has(key):
		var d := NativeCtor.defaults(cls).duplicate()
		d["__native"] = cls
		d["__path"] = ""
		_cache[key] = d
	return _cache[key]

static func class_defaults(bp_path: String) -> Dictionary:
	var p := UePkg.strip(bp_path)
	if _cache.has(p):
		return _cache[p]
	var nat := native_class(UePkg.native_root(p))
	var d: Dictionary = UePkg._merge(NativeCtor.defaults(nat) if nat != "" else {}, UePkg.defaults(p))
	d["__native"] = nat
	d["__path"] = p
	_cache[p] = d
	return d

# Merged CDO TMap property `prop` of `bp_path` as {UE key: value}. The chain is merged key by key, root first:
# a child CDO lists only the pairs it changed (BP_2HSwordAnimationProfile_Greatsword's Attacks holds only
# LeftStrike/RightStrike while its parent BP_2HSwordAnimationProfile holds all six moves).
# UNCONFIRMED: that UE 4.26 serializes TMap CDO properties as per-key deltas is engine behaviour, inferred from the data.
static func map_prop(bp_path: String, prop: String) -> Dictionary:
	var out := {}
	var ch := UePkg.chain(UePkg.strip(bp_path))
	for i in range(ch.size() - 1, -1, -1):
		for kv in UePkg.cdo(UePkg.load_pkg(ch[i])).get(prop, []):
			out[kv.Key] = kv.Value
	return out

# One weapon's combat setup: WeaponData (data builder), motion Blueprint per EAttackMove, parry motion Blueprint.
# Move -> motion class is UMotionSystemComponent::GetAttackMotionClass rva=0x14bc010: the weapon's
# GetRelevantMeleeWeaponAnimationProfile()->Attacks map, keyed by EAttackMove; a missing key falls back to
# UAttackMotion::StaticClass (here: "" = native UAttackMotion defaults).
class WeaponSetup:
	var weapon: WeaponData
	var equip: EquipmentDef
	var profile := ""			# UMeleeWeaponAnimationProfile Blueprint
	var motions := {}			# EAttackMove -> motion Blueprint path
	var parry_motion := ""		# "" = native UParryMotion

static func weapon_setup(weapon_path: String) -> WeaponSetup:
	var out := WeaponSetup.new()
	out.weapon = UeWeapon.load_weapon(weapon_path)
	out.equip = EquipmentDef.load_def(weapon_path)
	var prof := profile_motions(out.equip.weapon_animation_profile if out.equip != null else "")
	out.profile = prof.profile
	out.motions = prof.motions
	out.parry_motion = prof.parry_motion
	return out

# One animation profile's motion classes (UMeleeWeaponAnimationProfile Attacks map and ParryMotion)
class ProfileMotions:
	var profile := ""
	var motions := {}			# EAttackMove -> motion Blueprint path
	var parry_motion := ""		# "" = none set (GetParryMotionClass keeps the previous choice)

static func profile_motions(profile_path: String) -> ProfileMotions:
	var out := ProfileMotions.new()
	out.profile = UePkg.strip(profile_path)
	if out.profile == "":
		return out
	var attacks := map_prop(out.profile, "Attacks")
	var r := UeRec.new(attacks, out.profile + ".Attacks")
	for k in attacks:
		var mv := CombatEnums.move_from_ue(String(k))
		if mv < 0:
			r.errors.append("%s.Attacks: unknown EAttackMove %s" % [out.profile, k])
			continue
		var bp := r.ref_path(attacks[k], String(k))
		if bp != "":
			out.motions[mv] = bp
	var cdo := UeRec.new(UePkg.defaults(out.profile), out.profile)
	out.parry_motion = cdo.obj("ParryMotion")		# TSubclassOf: absent = nullptr
	if not r.ok() or not cdo.ok():
		r.done(null)
		cdo.done(null)
	return out

# Motion class defaults as a typed record (MotionDefs), cached per Blueprint / native class
static var _motion_defs := {}

static func motion_def(bp_path: String) -> MotionDefs.Base:
	var key := UePkg.strip(bp_path)
	if not _motion_defs.has(key):
		_motion_defs[key] = MotionDefs.from_ue(class_defaults(key))
	return _motion_defs[key]

static func native_motion_def(cls: String) -> MotionDefs.Base:
	var key := "native:" + cls
	if not _motion_defs.has(key):
		_motion_defs[key] = MotionDefs.from_ue(native_only(cls))
	return _motion_defs[key]

# AMordhauCharacter KickWeapon (+0x1210, AKickWeapon): BP_MordhauCharacter's Blueprint variable KickClass
# (extract/decomp/.../Characters/BP_MordhauCharacter.BP_MordhauCharacter_C.cpp line 8: BP_KickWeapon), spawned by its
# SpawnKickWeapon event. UNCONFIRMED: that SpawnKickWeapon stores the spawned actor in KickWeapon (the ubergraph's
# bytecode is not in the package dump; its frame holds K2Node_DynamicCast_AsKick_Weapon). UKickMotion::FindWeapon
# rva=0x165b4b0 returns Character->KickWeapon for every kick.
static func kick_weapon_path() -> String:
	return object_path(class_defaults(CHARACTER).get("KickClass"))

# a CDO object reference {"ObjectPath": ...} -> stripped package path ("" for none)
static func object_path(ref) -> String:
	return UePkg.strip(String(ref.get("ObjectPath", ""))) if ref is Dictionary else ""

# AMordhauWeapon::GetBaseAttackInfo rva=0x1620c50: Kick -> KickAttack, Stab/AltStab -> StabAttack, Couch ->
# CouchAttack, Bash -> BashAttack, anything else (the strikes) -> StrikeAttack.
static func attack_info_for(w: WeaponData, move: int) -> AttackInfo:
	match move:
		CombatEnums.Move.KICK: return w.kick
		CombatEnums.Move.STAB, CombatEnums.Move.ALT_STAB: return w.stab
		CombatEnums.Move.COUCH: return w.couch
		CombatEnums.Move.BASH: return w.bash
	return w.strike

# A CDO FAttackInfo struct (mdx json Dictionary) -> AttackInfo: fields mapped by AttackInfo.MAP; UE keys the map
# does not know go to .unmapped, MAP keys the struct does not carry to .unset (they keep the FAttackInfo ctor value).
static func attack_info_from_ue(d: Dictionary) -> AttackInfo:
	var a := AttackInfo.new()
	a.unmapped = UePkg.assign(a, AttackInfo.MAP, d)
	for k in AttackInfo.MAP:
		if not d.has(k):
			a.unset.append(k)
	return a

# AMordhauCharacter BlockCollider (UBoxComponent) of BP_MordhauCharacter, the component template's CDO values, in
# Godot metres in the actor frame (UePhysics.swap x 0.01). BoxExtent (65, 65, 105) is already a half extent
# (UBoxComponent). UNCONFIRMED: Low/HighBlockColliderRelativeOffset (AMordhauCharacter, moved with crouch / look) are
# not applied, the template's RelativeLocation is. Both are serialized on the template; a template without them is
# an error (the UBoxComponent / USceneComponent ctor values are not decoded here).
class BlockCollider:
	var half := Vector3.ZERO		# BoxExtent
	var centre := Vector3.ZERO		# RelativeLocation

static func block_collider() -> BlockCollider:
	if _cache.has("block_collider"):
		return _cache.block_collider
	var out := BlockCollider.new()
	var r := UeRec.new(UePkg.export_named(UePkg.load_pkg(CHARACTER), "BlockColliderBP_GEN_VARIABLE").get("Properties", {}),
		"BP_MordhauCharacter.BlockCollider")
	out.half = UePhysics.swap(r.sub("BoxExtent").d) * 0.01
	out.centre = UePhysics.swap(r.sub("RelativeLocation").d) * 0.01
	r.v3("BoxExtent")
	r.v3("RelativeLocation")
	_cache.block_collider = r.done(out)
	return _cache.block_collider

# UCurveFloat keys from mdx json (FloatCurve.Keys).
static func curve_keys(obj_path: String) -> Array:
	var e := UePkg.export_of(UePkg.load_pkg(UePkg.strip(obj_path)), "CurveFloat")
	return e.get("Properties", {}).get("FloatCurve", {}).get("Keys", [])

# FRichCurve::Eval (engine code compiled into the exe; .text 0x3023860 = rva 0x3024860, disassembled with capstone,
# label from extract/native/labels_all.tsv) and EvalForTwoKeys (rva 0x3024fa0). FRichCurveKey (0x1c bytes): InterpMode
# +0 (RCIM_Linear 0, RCIM_Constant 1, RCIM_Cubic 2), TangentWeightMode +2, Time +4, Value +8, ArriveTangent +0xc,
# LeaveTangent +0x14.
#   t <= first key: PreInfinityExtrap == Linear (3) -> line through the first two keys, else first value
#   t >= last key:  PostInfinityExtrap == Linear -> line through the last two keys, else last value
#   else the two keys around t (binary search): dt = t2 - t1 <= 0 or Constant -> v1; Linear -> lerp;
#     Cubic, unweighted tangents -> Bezier(v1, v1 + Leave1*dt/3, v2 - Arrive2*dt/3, v2) at (t - t1)/dt
#     (the 1/3 is _DAT_14406a414)
# Not ported: weighted tangents (call 0x143042930), cycle/oscillate extrapolation (modes 0..2).
# UNCONFIRMED: a curve JSON without Pre/PostInfinityExtrap is RCCE_Constant (the FRealCurve default constructor is
# inlined and not located); for the combat curves t stays inside the key range, where extrapolation does not apply.
const _INTERP := {"RCIM_Linear": 0, "RCIM_Constant": 1, "RCIM_Cubic": 2}

static func _two_keys(a: Dictionary, b: Dictionary, t: float) -> float:
	var dt: float = b.Time - a.Time
	var mode: int = _INTERP.get(String(a.get("InterpMode", "RCIM_Linear")), 0)
	if dt <= 0.0 or mode == 1:
		return a.Value
	var alpha: float = (t - a.Time) / dt
	if mode == 0:
		return (b.Value - a.Value) * alpha + a.Value
	var wa := String(a.get("TangentWeightMode", "RCTWM_WeightedNone"))
	var wb := String(b.get("TangentWeightMode", "RCTWM_WeightedNone"))
	if not (wa in ["RCTWM_WeightedNone", "RCTWM_WeightedArrive"] and wb in ["RCTWM_WeightedNone", "RCTWM_WeightedLeave"]):
		push_error("CombatData.curve_eval: weighted tangents not ported")
		return NAN
	var third := UeRdata.f32(0x14406a414)	# 1/3, engine curve code (reader-internal)
	var p0: float = a.Value
	var p1: float = p0 + float(a.get("LeaveTangent", 0.0)) * dt * third
	var p3: float = b.Value
	var p2: float = p3 - float(b.get("ArriveTangent", 0.0)) * dt * third
	# de Casteljau, same operation order as the disassembly
	var p01 := lerpf(p0, p1, alpha)
	var p12 := lerpf(p1, p2, alpha)
	var p23 := lerpf(p2, p3, alpha)
	return lerpf(lerpf(p01, p12, alpha), lerpf(p12, p23, alpha), alpha)

static func curve_eval(keys: Array, t: float, pre_extrap := "RCCE_Constant", post_extrap := "RCCE_Constant") -> float:
	var n := keys.size()
	if n == 0:
		return 0.0
	var first: Dictionary = keys[0]
	if n < 2 or t <= first.Time:
		if pre_extrap == "RCCE_Linear" and n > 1:
			var d: float = keys[1].Time - first.Time
			if absf(d) > UeRdata.f32(0x144014a88):
				return (keys[1].Value - first.Value) / d * (t - first.Time) + first.Value
		return first.Value
	var last: Dictionary = keys[n - 1]
	if t >= last.Time:
		if post_extrap == "RCCE_Linear":
			var d2: float = keys[n - 2].Time - last.Time
			if absf(d2) > UeRdata.f32(0x144014a88):
				return (keys[n - 2].Value - last.Value) / d2 * (t - last.Time) + last.Value
		return last.Value
	var lo := 1
	var cnt := n - 2
	while cnt > 0:
		var half := cnt >> 1
		if t >= float(keys[lo + half].Time):
			lo = lo + half + 1
			cnt -= half + 1
		else:
			cnt = half
	return _two_keys(keys[lo - 1], keys[lo], t)

# FloatCurve extrapolation modes of a curve package (absent -> RCCE_Constant, see above)
static func curve_extrap(obj_path: String) -> Array:
	var fc: Dictionary = UePkg.export_of(UePkg.load_pkg(UePkg.strip(obj_path)), "CurveFloat").get("Properties", {}).get("FloatCurve", {})
	return [String(fc.get("PreInfinityExtrap", "RCCE_Constant")).get_slice("::", fc.get("PreInfinityExtrap", "").count("::")),
		String(fc.get("PostInfinityExtrap", "RCCE_Constant")).get_slice("::", fc.get("PostInfinityExtrap", "").count("::"))]

# UCurveFloat::GetFloatValue (rva 0x3026f70; the func_0x143026f70 calls in the Ghidra C) on a curve package
static func curve_value(obj_path: String, t: float) -> float:
	var ex := curve_extrap(obj_path)
	return curve_eval(curve_keys(obj_path), t, ex[0], ex[1])

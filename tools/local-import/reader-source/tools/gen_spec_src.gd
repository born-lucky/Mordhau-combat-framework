# gen_spec_src.gd - dump the typed records the port reads (components/ue readers: UeWeapon, CombatData, MotionDefs,
# CharacterData, ModeData, BotData, UeWearable) as plain JSON, the input of scripts/sheets_populate.py (the Spreadsheet
# Method workbook sheets/mordhau_spec.xlsx, docs/SPEC_SHEETS.md). Values are the readers' own output, nothing typed in:
# the workbook is a generated view of the same data the records give the game, which the equivalence test checks.
#   sh scripts/godot_import.sh --run timeout 900 godot --headless --path godot --script res://tools/gen_spec_src.gd
# Output: res://data_gen/spec_src/<kind>.json (gitignored: generated from the user's own game files).
# Each object is {"__class": <script class>, "__types": {field: Variant.Type}, field: value, ...}; floats are written at
# full precision so the workbook round-trips them exactly.
extends SceneTree

const OUT := "res://data_gen/spec_src"
const SKIP := {"skins": true, "unmapped": true, "resource_local_to_scene": true, "resource_path": true,
	"resource_name": true, "resource_scene_unique_id": true, "script": true}

func _init() -> void:
	DirAccess.make_dir_recursive_absolute(ProjectSettings.globalize_path(OUT))
	var t := Time.get_ticks_msec()
	var fails := 0
	# `-- --only=maps,anims` dumps just those kinds (each kind is one file; the others are left as they are)
	var only := PackedStringArray()
	for a in OS.get_cmdline_user_args():
		if a.begins_with("--only="):
			only = a.trim_prefix("--only=").split(",", false)
	var kinds := ["weapons", "motions", "character", "modes", "bots", "perks", "rules", "wearables", "ai_trees",
		"settings", "customization", "anims", "maps", "modes_extra", "cosmetics", "audio", "motions_raw", "horde"]
	for k in kinds:
		if not only.is_empty() and not only.has(k):
			continue
		var data = call("_" + k)
		fails += _save(k, data)
		print("%s done at %d ms" % [k, Time.get_ticks_msec() - t])
		UePkg.clear_cache()
	quit(1 if fails else 0)

# Motion properties the typed MotionDefs records do not carry yet but the Rust combat port reads (rust-combat request,
# sheets r7): CheckAttackParry rva=0x1616cd0, CheckClash rva=0x16179a0, CheckParry rva=0x164ed40. Values = the merged
# native ctor + CDO chain (CombatData.class_defaults; UAttackMotion ctor rva=0x1612540), offsets per
# extract/native/types/UAttackMotion.h / UParryMotion.h. Plus each weapon's ClashCollider component template (merged).
const MOTION_RAW := ["MaxParryAngleForChamberAndActiveParry", "MaxParryWeaponAngleForChamberAndActiveParry",
	"ActiveParryStaminaCost", "ActiveParryWindow", "bNoDamageInEarlyRelease", "bCanBeParriedByForwardCollider",
	"bCanBeParriedByForwardColliderInEarlyRelease", "ClashAngle", "EarlyReleaseIsClashableAfter", "GlanceDamageModifier",
	"MaxParryAngle", "MaxParryWeaponAngle", "TimedBlockMemoryDuration", "HeldBlockMemoryDuration",
	"bCanTraceHitUsingShieldBlockCollider"]

func _motions_raw() -> Dictionary:
	var out := {"motions": {}, "clash_collider": {}}
	var mf := FileAccess.get_file_as_string(OUT + "/motions.json")
	var motions = JSON.parse_string(mf) if mf != "" else {}
	for key in motions:
		var d: Dictionary = CombatData.native_only(String(key).trim_prefix("native:")) if String(key).begins_with("native:") \
			else CombatData.class_defaults(key)
		var e := {}
		for k in MOTION_RAW:
			if d.has(k):
				e[k] = val(d[k])
		out.motions[key] = e
	for p in UeWeapon.list_weapons():
		var w := UeWeapon.load_weapon(p)
		var c := UeWeapon._component(w.chain, "ClashCollider")
		if not c.is_empty():
			out.clash_collider[w.id] = val(c)
	return out

# a class's merged defaults (native ctor + Blueprint CDO chain, CombatData.class_defaults) as plain JSON values
func _cdo(path: String) -> Dictionary:
	var d := CombatData.class_defaults(path)
	var out := {}
	for k in d:
		out[str(k)] = val(d[k])
	return out

# Horde + Battle Royale: the classes each mode installs (merged defaults; their Blueprint bytecode is read by
# scripts/sheets_populate.py through scripts/kismet) and the Horde squad data assets (USquadInfo)
# Horde + Battle Royale records exactly as the reference reads them for mh-mode (rust-mode-ai request, sheets r10):
# the golden mode exporter's own readers (godot/tools/golden/mode.gd _horde_data / _br_data: config, squad waves,
# SquadInfo assets over the USquadInfo ctor rva=0x166fb50, enemies + KillReward, FC_HordeDamageModifier samples, the
# extras: graves, chests, purchasables, buy menus, merchant, skill tree, Demon Invasion), the two modes'
# MordhauModeData (metadata, scoring, state incl. team colours / auto assign, starts) and E_HordeSkill's Names (the
# UserDefinedEnum's ordered name -> byte value table)
func _horde() -> Dictionary:
	var gm = load("res://tools/golden/mode.gd").new()
	var hd := MordhauModeData.new("Mordhau/Content/Mordhau/Maps/DuelCamp/HRD_Camp", gm.HRD_GM, gm.HRD_GS, gm.HRD_META)
	var bd := MordhauModeData.new("Mordhau/Content/Mordhau/Maps/FeitoriaMap/BR_Feitoria", gm.BR_GM, gm.BR_GS, gm.BR_META)
	var en := UePkg.export_of(UePkg.load_pkg("Mordhau/Content/Mordhau/Blueprints/GameModes/Horde/E_HordeSkill"), "UserDefinedEnum")
	# + each mode's combat ModeRules (the same CombatData.mode_rules reader the six modes' "rules" use; rust-mode-ai r4)
	return {"horde": val(gm._horde_data()), "br": val(gm._br_data()), "hrd_mode": obj(hd), "br_mode": obj(bd),
		"hrd_rules": obj(CombatData.mode_rules(gm.HRD_GM, gm.HRD_GS)), "br_rules": obj(CombatData.mode_rules(gm.BR_GM, gm.BR_GS)),
		"horde_skill_names": val(en.get("Names", {})),
		"curves": {"damage": "Mordhau/Content/Mordhau/Blueprints/GameModes/Horde/FC_HordeDamageModifier"}}

func _modes_extra() -> Dictionary:
	const GM := "Mordhau/Content/Mordhau/Blueprints/GameModes/"
	var out := {"classes": {}, "squads": {}}
	for p in [GM + "Horde/BP_HordeGameMode", GM + "Horde/BP_HordeGameState", GM + "Horde/BP_HordePlayerController",
			GM + "Horde/BP_HordePlayerState", GM + "BattleRoyale/BP_BattleRoyaleGameMode",
			GM + "BattleRoyale/BP_BattleRoyaleGameState"]:
		if UePkg.exists(p):
			out.classes[p] = _cdo(p)
	for p in UePkg.packages_of_class(GM + "Horde", ["SquadInfo"]):
		var e := UePkg.export_of(UePkg.load_pkg(p), "SquadInfo")
		out.squads[p] = val(e.get("Properties", {}))
	return out

# cosmetics: weapon skins (UeWeapon skins, per weapon), BP_MordhauSingleton colour tables / emblems / badges / voices /
# faces, each referenced class's merged defaults (scalar fields only)
func _cosmetics() -> Dictionary:
	var out := {"skins": {}, "singleton": {}, "classes": {}}
	for p in UeWeapon.list_weapons():
		var w := UeWeapon.load_weapon(p)
		if not w.skins.is_empty():
			out.skins[w.id] = val(w.skins)
	var s := UeWearable.singleton()
	for k in ["ColorTables", "SkinColorTable", "EyeColorTable", "HairColorTable", "EmblemColorTable", "MetalTintsColorTable",
			"Emblems", "Badges", "MaleVoices", "FemaleVoices", "MaleFaces", "FemaleFaces", "Eyebrows", "Archetypes"]:
		out.singleton[k] = val(s.get(k))
	# colour tables: every entry's colour as the customization reader resolves it (UeWearable.color)
	out["colors"] = []
	var tables = s.get("ColorTables", [])
	for ti in (tables.size() if tables is Array else 0):
		var entries = tables[ti].get("Entries", []) if tables[ti] is Dictionary else []
		var row := []
		for ei in entries.size():
			row.append({"class": UePkg.strip(String(entries[ei].get("ObjectPath", ""))), "color": val(UeWearable.color(ti, ei))})
		out.colors.append(row)
	for k in ["Emblems", "Badges", "MaleVoices", "FemaleVoices", "Eyebrows"]:
		var arr = s.get(k, [])
		for ref in (arr if arr is Array else []):
			var p := UeWearable.pkg_path(ref)
			if p != "" and not out.classes.has(p) and UePkg.exists(p):
				var d := _cdo(p)
				var flat := {}
				for f in d:
					if typeof(d[f]) in [TYPE_BOOL, TYPE_INT, TYPE_FLOAT, TYPE_STRING] or (d[f] is Dictionary and d[f].has("ObjectPath")):
						flat[f] = d[f]
				out.classes[p] = flat
	return out

# every SoundCue (UeSound.cue: graph, waves, volume / pitch, attenuation), SoundAttenuation and SoundConcurrency asset
func _audio() -> Dictionary:
	var out := {"cues": {}, "attenuation": {}, "concurrency": {}}
	var n := 0
	for p in UePkg.packages_of_class("Mordhau/Content", ["SoundCue"]):
		out.cues[p] = val(UeSound.cue(p))
		n += 1
		if n % 100 == 0:
			UePkg.clear_cache()
	for p in UePkg.packages_of_class("Mordhau/Content", ["SoundAttenuation"]):
		out.attenuation[p] = val(UeSound.attenuation(p))
	for p in UePkg.packages_of_class("Mordhau/Content", ["SoundConcurrency"]):
		out.concurrency[p] = val(UePkg.export_of(UePkg.load_pkg(p), "SoundConcurrency").get("Properties", {}))
	return out

func _perks() -> Dictionary:
	# UPerkSystemComponent: native ctor defaults (every perk effect parameter, extract/native/types/UPerkSystemComponent.h)
	# with BP_MordhauCharacter's PerkSystemComponent template merged over them when it has one
	var d: Dictionary = UePkg._merge(NativeCtor.defaults("UPerkSystemComponent"), CharacterData.subobject("PerkSystemComponent"))
	var effects := {}
	for k in d:
		if not String(k).begins_with("__"):
			effects[k] = val(d[k])
	return {"perks": UeWearable.perks(), "perk_system": obj(CharacterData.perks()), "perk_effects": effects,
		"character_points": UeWearable.character_points()}

# behaviour trees: every node of every tree (BotData.tree_def, the reader the AI port runs)
func _ai_trees() -> Dictionary:
	var out := {}
	# Consumer importer enumerates the installed paks, without an exported JSON directory.
	for package in UePkg.packages_of_class(BotData.BT_ROOT, ["BehaviorTree"]):
		var name := String(package).get_file()
		var td := BotData.tree_def(name)
		if td != null:
			out[name] = obj(td, -200)
	return out

# UMordhauGameUserSettings::SetToDefaults + CVar defaults (game/ui/user_settings.gd, ui r1)
func _settings() -> Dictionary:
	var u := UserSettings.new()
	return {"v": val(u.v), "cvars": val(u.cvars), "version": u.version, "mordhau_version": u.mordhau_version}

# loadout rules (CustomizationData: FCharacterProfile / FCharacterGearCustomization validation inputs)
func _customization() -> Dictionary:
	var eq := {}
	for i in CustomizationData.equipment_count():
		var r := CustomizationData.equipment(i)
		if r != null:
			eq[str(i)] = obj(r)
	return {"equipment": eq, "perk_costs": val(CustomizationData.perk_costs()),
		"character_points": CustomizationData.character_points()}

# animation assets the motions / weapons reference: kind, length, montage blend + segments, notifies (raw)
func _anims() -> Dictionary:
	var paths := {}
	for kind in ["motions", "weapons", "character"]:
		var t := FileAccess.get_file_as_string(OUT + "/" + kind + ".json")
		for m in RegEx.create_from_string("\"(Mordhau/Content/[^\"]+)\"").search_all(t):
			paths[UePkg.strip(m.get_string(1))] = true
	var out := {}
	for p in paths:
		if not UePkg.exists(p):
			continue
		var kind := AnimData.asset_kind(p)
		if kind == "":
			continue
		var e := {"kind": kind}
		if kind == "AnimMontage":
			e["montage"] = obj(AnimData.montage(p), -4)
			e["notifies"] = val(AnimData.export_of(p, "AnimMontage").get("Properties", {}).get("Notifies", []))
		elif kind == "AnimSequence":
			e["length"] = AnimData.sequence_length(p)
			e["additive"] = AnimData.is_additive(p)
			e["notifies"] = val(AnimData.export_of(p, "AnimSequence").get("Properties", {}).get("Notifies", []))
		out[p] = e
	return out

# per-map actors of every game-mode map package (manifest: Maps/**/<PREFIX>_*.umap): player starts, control points,
# nav mesh bounds; nav agent (DefaultEngine.ini) once
func _maps() -> Dictionary:
	var out := {"__nav_agent": obj(ModeData.nav_agent())}
	var rx := RegEx.create_from_string("^(Mordhau/Content/Mordhau/Maps/.*/[A-Z]{2,4}_[^/]*)\\.umap$")
	var maps := []
	# Keep the same package-name filter, now applied to actual mounted pak paths.
	for path in UePakVfs.list():
		var m := rx.search(String(path))
		if m != null:
			maps.append(m.get_string(1))
	maps.sort()
	for p in maps:
		var e := {"package": p}
		e["player_starts"] = val(ModeData.player_starts(p))
		e["control_points"] = val(ModeData.control_points(p))
		e["nav_bounds"] = val(ModeData.nav_bounds(p))
		out[p.get_file()] = e
		UePkg.clear_cache()
	return out

func _save(kind: String, data) -> int:
	var f := FileAccess.open("%s/%s.json" % [OUT, kind], FileAccess.WRITE)
	if f == null:
		push_error("open %s" % kind)
		return 1
	f.store_string(JSON.stringify(data, " ", true, true))
	f.close()
	return 0

# any record value -> JSON value
func val(v, depth := 0):
	match typeof(v):
		TYPE_NIL, TYPE_BOOL, TYPE_INT, TYPE_FLOAT, TYPE_STRING:
			return v
		TYPE_STRING_NAME, TYPE_NODE_PATH:
			return String(v)
		TYPE_VECTOR2, TYPE_VECTOR2I:
			return [v.x, v.y]
		TYPE_VECTOR3, TYPE_VECTOR3I:
			return [v.x, v.y, v.z]
		TYPE_COLOR:
			return [v.r, v.g, v.b, v.a]
		TYPE_TRANSFORM3D:
			return {"origin": val(v.origin), "x": val(v.basis.x), "y": val(v.basis.y), "z": val(v.basis.z)}
		TYPE_DICTIONARY:
			var d := {}
			for k in v:
				d[str(k)] = val(v[k], depth + 1)
			return d
		TYPE_OBJECT:
			if v == null:
				return null
			return obj(v, depth + 1) if depth < 4 else str(v)	# obj(x, -8) for deep trees
		TYPE_CALLABLE, TYPE_SIGNAL, TYPE_RID:
			return null
	if typeof(v) >= TYPE_ARRAY:
		var a := []
		for x in v:
			a.append(val(x, depth + 1))
		return a
	return str(v)

# a record object -> {"__class", "__types", field: value} over its script variables
func obj(o: Object, depth := 0):
	if o == null:
		return null
	var out := {}
	var types := {}
	var sc: Script = o.get_script()
	out["__class"] = String(sc.get_global_name()) if sc != null and sc.get_global_name() != &"" else o.get_class()
	if sc != null and out["__class"] == o.get_class():		# inner class (MotionDefs.Attack): the script file + the native base
		out["__class"] = sc.resource_path.get_file().get_basename() + ":" + _inner_name(o)
	for p in o.get_property_list():
		if not (int(p.usage) & PROPERTY_USAGE_SCRIPT_VARIABLE) or SKIP.has(p.name):
			continue
		types[p.name] = int(p.type)
		out[p.name] = val(o.get(p.name), depth)
	out["__types"] = types
	return out

# MotionDefs inner classes have no global name: tell them apart by type, most derived first
func _inner_name(o: Object) -> String:
	if o is MotionDefs.Kick: return "Kick"
	if o is MotionDefs.Attack: return "Attack"
	if o is MotionDefs.Parry: return "Parry"
	if o is MotionDefs.Feinted: return "Feinted"
	if o is MotionDefs.Blocked: return "Blocked"
	if o is MotionDefs.Flinch: return "Flinch"
	if o is MotionDefs.Stun: return "Stun"
	if o is MotionDefs.Disarmed: return "Disarmed"
	if o is MotionDefs.Base: return "Base"
	return "inner"

func _weapons() -> Dictionary:
	var out := {}
	for p in UeWeapon.list_weapons():
		var s := CombatData.weapon_setup(p)
		var e := {"weapon": obj(s.weapon), "equip": obj(s.equip), "profile": s.profile, "parry_motion": s.parry_motion,
			"motions": {}}
		for m in s.motions:
			e.motions[CombatEnums.Move.find_key(m)] = s.motions[m]
		if s.equip != null and s.weapon.b_has_alternate_mode:
			var alt := CombatData.profile_motions(s.equip.switched().weapon_animation_profile)
			e["alt_profile"] = alt.profile
			e["alt_parry_motion"] = alt.parry_motion
			e["alt_motions"] = {}
			for m in alt.motions:
				e.alt_motions[CombatEnums.Move.find_key(m)] = alt.motions[m]
		out[s.weapon.id] = e
	return out

func _motions() -> Dictionary:
	var bps := {CombatData.FEINTED_MOTION: true, CombatData.BLOCKED_MOTION: true, CombatData.FLINCH_MOTION: true,
		CombatData.STUN_MOTION: true, CombatData.DISARMED_MOTION: true}
	var wf := FileAccess.get_file_as_string(OUT + "/weapons.json")
	var weapons = JSON.parse_string(wf) if wf != "" else {}
	for id in weapons:
		var e: Dictionary = weapons[id]
		for k in ["motions", "alt_motions"]:
			for m in e.get(k, {}):
				bps[e[k][m]] = true
		for k in ["parry_motion", "alt_parry_motion"]:
			if String(e.get(k, "")) != "":
				bps[e[k]] = true
	var out := {}
	for bp in bps:
		var d := CombatData.motion_def(bp)
		if d != null:
			out[bp] = obj(d)
	for cls in ["UAttackMotion", "UParryMotion", "UIdleMotion"]:
		var d := CombatData.native_motion_def(cls)
		if d != null:
			out["native:" + cls] = obj(d)
	return out

func _character() -> Dictionary:
	# the movement model's own load (MordhauMovement.load_default: the records + DefaultEngine.ini physics), so the
	# spec carries everything the movement port reads; curves = the UCurveFloat keys + extrapolation it samples
	# (CombatData.curve_keys / curve_extrap), as godot/tools/export_golden_character.gd dumps them
	var m0 := MordhauMovement.load_default()
	var curves := {}
	for path in [m0.fx.turn_sprint_prevention_decay_curve, m0.fx.turn_sprint_prevention_slowdown_curve]:
		if path == "":
			continue
		var ks := []
		for k in CombatData.curve_keys(path):
			ks.append({"time": float(k.Time), "value": float(k.Value), "interp": String(k.get("InterpMode", "RCIM_Linear"))})
		curves[path] = {"keys": ks, "extrap": CombatData.curve_extrap(path)}
	return {"character": obj(m0.ch), "movement": obj(m0.cfg),
		"move_extra": obj(m0.fx), "camera": obj(CharacterData.camera()),
		"stamina": obj(CharacterData.stat("UStaminaStatComponent")), "health": obj(CharacterData.stat("UHealthStatComponent")),
		"kick_weapon": CombatData.kick_weapon_path(),
		"physics": {"gravity_z": m0.gravity_z, "terminal_velocity": m0.terminal_velocity}, "curves": curves,
		# AMordhauCharacter block-collider terms UpdateBlockCollider rva=0x15700a0 reads (rust-combat request, sheets r9):
		# merged native ctor + BP_MordhauCharacter CDO (CombatData.class_defaults); FTransforms as UE dicts
		"character_raw": _pick(CombatData.class_defaults(CombatData.CHARACTER), ["BlockColliderForwardParryDistance",
			"LowBlockColliderRelativeOffset", "HighBlockColliderRelativeOffset"])}

func _pick(d: Dictionary, keys: Array) -> Dictionary:
	var out := {}
	for k in keys:
		if d.has(k):
			out[k] = val(d[k])
	return out

func _modes() -> Dictionary:
	var out := {}
	var datas := {"FFA": FfaMode.FfaData.shared(), "TDM": TdmMode.TdmData.shared_tdm(), "SKM": SkmMode.SkmData.shared(),
		"DU": DuelMode.DuelData.shared(), "TF": TfMode.TfData.shared_tf(), "FL": FlMode.FlData.new()}
	var table := {}
	for r in ModeTable.rows():
		table[r.id] = {"map": r.map, "fallback": r.fallback, "rooms": r.rooms, "bots": r.bots}
	for id in datas:
		var d = datas[id]
		var meta := String(d.get_script().get_script_constant_map().get("METADATA", ""))
		var e := {"data": obj(d), "rules": obj(CombatData.mode_rules(d.game_mode, d.game_state)),
			"table": table.get(id, {}), "metadata": meta, "maps": Array(ModeData.metadata_maps(meta)) if meta != "" else []}
		out[id] = e
	return out

# Rule tables the port implements, read back from the port itself (not retyped): which FAttackInfo slot each EAttackMove
# takes (CombatData.attack_info_for = AMordhauWeapon::GetBaseAttackInfo), probed on a weapon whose attack slots are
# distinct objects, and the alternate-mode swap pairs (MotionSystem._ATTACK_SWAPS, OnRequestModeSwitch).
func _rules() -> Dictionary:
	var w := UeWeapon.load_weapon(UeWeapon.list_weapons()[0])
	var by_move := {}
	for mv in CombatEnums.Move:
		var ai := CombatData.attack_info_for(w, CombatEnums.Move[mv])
		for ue in WeaponData.ATTACKS:
			var slot: String = WeaponData.ATTACKS[ue]
			if ai != null and w.get(slot) == ai:
				by_move[mv] = slot
				break
	return {"attack_slot_by_move": by_move, "alt_mode_swaps": MotionSystem._ATTACK_SWAPS,
		"probe_weapon": w.id}

func _bots() -> Dictionary:
	var out := {}
	var dir := UePkg.root() + "/json/" + BotData.PROFILE_ROOT
	for f in DirAccess.get_files_at(dir):
		if f.ends_with(".json"):
			var nm := f.get_basename()
			out[nm] = obj(BotData.profile(nm))
	return out

func _wearables() -> Dictionary:
	var out := {}
	for p in UeWearable.list_wearables():
		var w := UeWearable.load_wearable(p)
		if w != null:
			out[w.id] = obj(w)
	UeWearable.clear_cache()
	return out

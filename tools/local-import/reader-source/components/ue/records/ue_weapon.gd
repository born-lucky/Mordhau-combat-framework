# ue_weapon.gd - Mordhau weapon Blueprint (mdx json) -> WeaponData.
# A "weapon" is a Blueprint whose SuperStruct chain ends at native MordhauWeapon, MordhauShield, FistsWeapon or
# KickWeapon: the four native roots whose CDO chains carry FAttackInfo properties (full scan of extract/json:
# 241 + 27 + 7 + 4 = 279 Blueprints). Ranged/thrown/misc items end at native MordhauEquipment and carry no
# FAttackInfo, so they are not WeaponData.
class_name UeWeapon

const NATIVE_ROOTS := ["MordhauWeapon", "MordhauShield", "FistsWeapon", "KickWeapon"]

# Native subclass constructors' stores into WeaponData fields, on top of the AMordhauWeapon defaults in weapon_data.gd
# (offsets per extract/native/types/AMordhauWeapon.h, AMordhauEquipment.h):
#  AFistsWeapon::AFistsWeapon rva 0x14e35d0: [0x19a8] AttackMask = 4, [0x19ac] ParryMask = 4, [0x56b] bIsTwoHanded = 1
#  AKickWeapon::AKickWeapon  rva 0x14e3890: [0x19a8] AttackMask = 2, [0x19ac] ParryMask = 2
#  AMordhauShield::AMordhauShield rva 0x15b61a0: [0xcb9] bCanAttack = 0 (also 0x1a0c bIsParryHeld = 0, the default)
#  AVirtualWeapon::AVirtualWeapon rva 0x166fa90 (base of Fists/Kick): [0xcb8] bAllowDrop = 0 (AVirtualWeapon.cpp:26)
const NATIVE_OVERRIDES := {
	"FistsWeapon": {"attack_mask": 4, "parry_mask": 4, "b_is_two_handed": true, "b_allow_drop": false},
	"KickWeapon": {"attack_mask": 2, "parry_mask": 2, "b_allow_drop": false},
	"MordhauShield": {"b_can_attack": false},
}

# Every weapon Blueprint package path, sorted. Candidates: packages whose first export class is BlueprintGeneratedClass
# or Function (UePkg.packages_of_class: the .uasset headers in the paks, or mdx list's extract/manifest.tsv). A Python
# full scan of all 35,730 JSON packages found the same 279 weapons, all inside that candidate set (200
# BlueprintGeneratedClass + 79 Function).
static func list_weapons() -> PackedStringArray:
	var cands := UePkg.packages_of_class("Mordhau/Content", ["BlueprintGeneratedClass", "Function"])
	var sup := {}			# package -> parent package, or "native:<Class>", or "" (not a class / unreadable)
	var out := PackedStringArray()
	for p in cands:
		if NATIVE_ROOTS.has(_root(p, sup)):
			out.append(p)
	out.sort()
	return out

# native root of p, memoised in sup; reads without UePkg's cache (thousands of packages)
static func _root(p: String, sup: Dictionary) -> String:
	var seen := []
	while true:
		if not sup.has(p):
			var s: Dictionary = UePkg.super_path(p)
			var op := String(s.get("ObjectPath", ""))
			sup[p] = UePkg.strip(op) if op.begins_with("Mordhau/Content") else \
				("native:" + String(s.get("ObjectName", "")).get_slice("'", 1) if op != "" else "")
		var n: String = sup[p]
		if n == "" or n.begins_with("native:"):
			return n.trim_prefix("native:")
		if seen.has(n):
			return ""
		seen.append(p)
		p = n
	return ""

# a weapon part class's AuxiliarySkeletalMesh (merged class defaults), as a package path
static func aux_mesh(part_class: String) -> String:
	return UePkg.text(UePkg.defaults(part_class).get("AuxiliarySkeletalMesh"))

# Part meshes of a weapon's default look as [UE package path, exported glb res path or ""]: its own SkeletalMesh, or
# the first part of each part type of its first skin plus that part's AuxiliarySkeletalMesh (HeldWeapon.build_node)
static func part_meshes(w: WeaponData) -> Array:
	var out := []
	if w.mesh != "":
		out.append([UePkg.strip(w.mesh), w.mesh_glb])
		return out
	var sk := skins(w)
	if sk.is_empty():
		return out
	for pt in sk[0].part_types:
		if pt.parts.is_empty():
			continue
		var part: SkinPart = pt.parts[0]
		if part.mesh != "":
			out.append([UePkg.strip(part.mesh), part.mesh_glb])
		var aux := aux_mesh(part.cls)
		if aux != "":
			out.append([UePkg.strip(aux), glb_for(aux)])
	return out

static func glb_for(mesh_path: String) -> String:
	if mesh_path == "":
		return ""
	var p := UePkg.strip(mesh_path)
	return "res://data/" + p + ".glb" if FileAccess.file_exists(UePkg.root() + "/gltf/" + p + ".glb") else ""

static func load_weapon(obj_path: String) -> WeaponData:
	var p := UePkg.strip(obj_path)
	var d := UePkg.defaults(p)
	var w := WeaponData.new()
	w.class_path = p
	w.id = p.get_file()
	w.chain = UePkg.chain(p)
	w.native_class = UePkg.native_root(p)
	var ov: Dictionary = NATIVE_OVERRIDES.get(w.native_class, {})
	for k in ov:
		w.set(k, ov[k])
	UePkg.assign(w, WeaponData.MAP, d)
	for k in WeaponData.ATTACKS:
		if d.has(k):
			w.set(WeaponData.ATTACKS[k], CombatData.attack_info_from_ue(d[k]))
		else:
			w.unset.append(k)
			if WeaponData.NATIVE_ATTACKS.has(k):
				w.set(WeaponData.ATTACKS[k], CombatData.attack_info_from_ue({}))	# native member: FAttackInfo ctor defaults
	for k in WeaponData.MAP:
		if not d.has(k):
			w.unset.append(k)
	w.mesh = UePkg.text(_component(w.chain, "SkeletalMeshComponent").get("SkeletalMesh"))
	w.mesh_glb = glb_for(w.mesh)
	for s in d.get("Skins", []):
		w.skins.append(_skin(s))
	return w

# A component template's properties merged down the chain (child's template's archetype is the parent's template).
static func _component(chain: PackedStringArray, name: String) -> Dictionary:
	var out := {}
	for i in range(chain.size() - 1, -1, -1):
		var props: Dictionary = UePkg.export_named(UePkg.load_pkg(chain[i]), name).get("Properties", {})
		for k in props:
			out[k] = UePkg._merge(out.get(k), props[k])
	return out

# FEquipmentSkinEntry (Skins[]). WeaponData stores them as plain dictionaries (it is saved as a .tres, data_gen):
# {name, icon, part_types: [{name, parts: [{class, name, mesh, mesh_glb}]}]}; game code reads them through the typed
# view skins(). Struct values hold only serialized fields: absent FText = empty, absent array = empty.
class SkinPart:
	var cls := ""			# part Blueprint class
	var name := ""			# its ItemName
	var mesh := ""			# its SkeletalMesh
	var mesh_glb := ""

class SkinPartType:
	var name := ""
	var parts: Array[SkinPart] = []

class WeaponSkin:
	var name := ""
	var icon := ""
	var part_types: Array[SkinPartType] = []

static func _skin(v) -> Dictionary:
	var r := UeRec.new(v, "FEquipmentSkinEntry")
	var types := []
	for t in r.arr_or("PartTypes"):
		var tr := UeRec.new(t, "FEquipmentPartType", r.errors)
		var parts := []
		for ref in tr.arr_or("Parts"):
			var cls := tr.ref_path(ref, "Parts[]")
			var mesh := ""
			var nm := ""
			if cls != "":
				var pd := UeRec.new(UePkg.defaults(cls), cls, r.errors)
				pd.obj("SkeletalMesh")					# validates the reference
				mesh = UePkg.text(pd.d.get("SkeletalMesh"))		# stored with its export index, as the .tres has it
				nm = pd.text_or("ItemName")
			parts.append({"class": cls, "name": nm, "mesh": mesh, "mesh_glb": glb_for(mesh)})
		types.append({"name": tr.text_or("PartName"), "parts": parts})
	var icon = r.d.get("SkinIcon")
	var out := {"name": r.text_or("SkinName"), "icon": UePkg.text(icon), "part_types": types}
	return out if r.done(true) != null else {}

# typed view of WeaponData.skins
static func skins(w: WeaponData) -> Array[WeaponSkin]:
	var out: Array[WeaponSkin] = []
	for s in w.skins:
		var ws := WeaponSkin.new()
		ws.name = s.name
		ws.icon = s.icon
		for t in s.part_types:
			var pt := SkinPartType.new()
			pt.name = t.name
			for p in t.parts:
				var sp := SkinPart.new()
				sp.cls = p["class"]
				sp.name = p.name
				sp.mesh = p.mesh
				sp.mesh_glb = p.mesh_glb
				pt.parts.append(sp)
			ws.part_types.append(pt)
		out.append(ws)
	return out

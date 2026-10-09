# ue_material.gd - build a Godot material for a UE material instance, from the CUE4Parse parameter file (mdx export,
# res://data/<package>.json) plus the package JSON of the instance and its Parent chain (extract/json, via UePkg).
#
# Parameters. The gltf-side file lists only the instance's own overrides ("Textures", "Colors", "Scalars"). Anything the
# instance inherits is read from the raw packages: each MaterialInstanceConstant has Parent, TextureParameterValues,
# VectorParameterValues, ScalarParameterValues and BasePropertyOverrides, and the root Material has the defaults in
# CachedExpressionData.Parameters (RuntimeEntries = scalars, [1] = vectors, [2] = textures, parallel to ScalarValues /
# VectorValues / TextureValues; checked on M_WeaponMaster, where entry [2] "AlbedoMap" pairs with Longsword_a).
# Lookups go nearest first: own file, own package, parent ... master. Only textures that were exported (.png present) count.
#
# Slots. Masters name their texture parameters differently, so slot names are matched by role (SLOT_*). Masters whose
# graph samples a texture directly (no parameter) appear with the texture's own name as the key; those are classified by
# the texture name's suffix. CUE4Parse's PM_* guesses come last (WoodWall_mat's PM_Diffuse is its packed _C texture).
#
# Packed layouts, measured on the exported PNGs (per-channel mean, % of texels at 0 / at 1):
#  RMA   *_RMA, *_RMAO, *_rmao, *_RMAE, *_r, weapon RoughnessMap: R roughness, G metallic (0/1 metal mask), B AO.
#        theFallen_rma G 20% 0; Matress_RMA G 100% 0; shieldsdecorative_rmao G 85% 0; WdMtlRpe_Trim_RMAO G 63% 0.
#  RHAO  slots RHAO, RHAO_0n, RoughnessAO; *_RHAO, *_RCrvAO: R roughness (0.74-0.96), G height or curvature, B AO
#        (0.81-0.95). No metal channel (metallic 0).
#  SRMH  slot SRMH, *_SRM: R specular (0.33-0.45), G roughness (0.22-0.77), B metallic (barrel_SRM 18% at 1, bag 100% 0).
#  MRA   slot Compact, *_C (-StandardShader_Mat): R metallic (Bucket_C 58% 0 / 4% 1, others all 0), G roughness (0.55-0.98),
#        B AO. Inferred from these statistics only.
#  R     "Roughness Texture" (Megascans *_Roughness, greyscale): roughness in R.
#
# Blend mode: BasePropertyOverrides.BlendMode of the nearest instance with bOverride_BlendMode, else the master's
# BlendMode, else BLEND_Opaque. The gltf-side "BlendMode" is not used when the packages are readable: CUE4Parse writes the
# instance's own value (bag.json says 0, its master bese_material is BLEND_Masked). BLEND_Masked -> alpha scissor at OpacityMaskClipValue (default 0.3333), BLEND_Translucent -> alpha
# blend. TwoSided likewise -> cull_disabled. UE: https://docs.unrealengine.com/4.26/en-US/RenderingAndGraphics/Materials/MaterialProperties/BlendModes/
# The opacity is taken from the albedo alpha only if at least 1% of its texels pass the clip value.
# wood__nails_basecolorw_a (Plks_Nails_1to1, masked) has 0.07% and would vanish; foliage cards have 4-73%.
#
# No albedo texture: base_color = the first of BASE_COLORS the material sets. Round 2 used vertex color to pick Color_R/G/B;
# that was wrong. The exported masters show those colors go with textures (MetalWood_Master samples WdMtlRpe_Trim_clrmask;
# RomanKit_Background samples romankit_background_basecolor and has UseGrungeR/G/B), and many Arena meshes have all-zero
# COLOR_0 (arena_mid_facade_arch_01a: 10048 vertices, all 0). Grunge tinting is not reproduced.
#
# Master math (round 4): the masters with a compiled-shader dump (docs/SHADERS.md: M_WeaponMaster, MetalWood_Master,
# Castle_Mat_Optimized_ItBump_Arena_New2, Arena_Blend_M, Architecture_M_New, RomanKit_Background, M_CY_RoundBanner, the
# wearable master) get their own shader mode (MODES); the generic path above stays for the rest.
#
# Masters export (round 3): scripts/mdx.sh exportlist with state/materials_r3_export.txt (111 master materials and
# textures the chains reference that were not yet exported).
#
# Why a ShaderMaterial, not StandardMaterial3D: UE normal maps are DirectX style and must be Y-flipped
# (https://docs.godotengine.org/en/stable/tutorials/3d/standard_material_3d.html#normal-map), which BaseMaterial3D
# cannot do (https://docs.godotengine.org/en/stable/classes/class_basematerial3d.html); the per-file import option
# process/normal_map_invert_y lives in .import files under extract/. Weapon albedo is grey and colored by ColorMap.
class_name UeMaterial

# godot/data is a junction to extract/gltf. Texture ObjectPaths are content paths below it.
const DATA := "res://data/"
const SHADER := "res://game/render/ue_tint.gdshader"
const SHADER_INC := "res://game/render/ue_tint.gdshaderinc"   # the code (all formulas); the .gdshader files include it

const SLOT_ALBEDO := ["AlbedoMap", "Albedo", "Albedo Texture", "BaseColor", "BaseColor_01", "Diffuse", "DiffuseMap",
	"Torso Albedo", "Color", "AlbedoDetail", "Vegetation_Color_T_Main"]
const SLOT_NORMAL := ["NormalMap", "Normal", "Normal Texture", "Normal_01", "Vegetation_Normal_T_Main"]
const SLOT_PACKED := {
	"RoughnessMap": "RMA", "RHAO": "RHAO", "RHAO_01": "RHAO", "RoughnessAO": "RHAO", "SRMH": "SRMH",
	"Compact": "MRA", "Roughness Texture": "R",
}
# Texture-name suffix (lower case) -> role, for slots that are texture names.
const SUFFIX := [
	["_rhao", "RHAO"], ["_rcrvao", "RHAO"], ["_srmh", "SRMH"], ["_srm", "SRMH"],
	["_rmae", "RMA"], ["_rmao", "RMA"], ["_rma", "RMA"], ["_r", "RMA_R"], ["_c", "MRA"],
	["_normal", "N"], ["_n", "N"],
	["_basecolor", "A"], ["_albedo", "A"], ["_diffuse", "A"], ["_color", "A"], ["_d", "A"], ["_a", "A"],
]
# Not albedo, even when nothing else matches.
const NOT_ALBEDO := ["grunge", "noise", "mask", "bloodmask", "_id", "translucency", "emblem", "pattern", "_s", "blendfunc",
	"default", "subsurface"]
# [rough_sel, metal_sel, ao_sel, rough_add, metal_add, ao_add]
const LAYOUTS := {
	"RMA": [Vector4(1, 0, 0, 0), Vector4(0, 1, 0, 0), Vector4(0, 0, 1, 0), 0.0, 0.0, 0.0],
	"RHAO": [Vector4(1, 0, 0, 0), Vector4.ZERO, Vector4(0, 0, 1, 0), 0.0, 0.0, 0.0],
	"SRMH": [Vector4(0, 1, 0, 0), Vector4(0, 0, 1, 0), Vector4.ZERO, 0.0, 0.0, 1.0],
	"MRA": [Vector4(0, 1, 0, 0), Vector4(1, 0, 0, 0), Vector4(0, 0, 1, 0), 0.0, 0.0, 0.0],
	"R": [Vector4(1, 0, 0, 0), Vector4.ZERO, Vector4.ZERO, 0.0, 0.0, 1.0],
	# No packed texture: roughness from a "Roughness" scalar if set, else 0.5 (UE's default for an unconnected input).
	"": [Vector4.ZERO, Vector4.ZERO, Vector4.ZERO, 0.5, 0.0, 1.0],
}
const BASE_COLORS := ["Color", "ColorMultiply", "ColorOverlay", "Color_Wood", "diffuse color", "ColorA", "AlbedoColor",
	"Albedo", "Ceramic_Color", "Sub_Color", "WoodColorMultiply"]
const BLEND := {"BLEND_Opaque": 0, "BLEND_Masked": 1, "BLEND_Translucent": 2, "BLEND_Additive": 3, "BLEND_Modulate": 4}

static var _shaders := {}
static var _alpha_ok := {}

# Parsed JSON, or {} on failure.
# FileAccess.get_file_as_string: https://docs.godotengine.org/en/stable/classes/class_fileaccess.html#class-fileaccess-method-get-file-as-string
# JSON.parse_string: https://docs.godotengine.org/en/stable/classes/class_json.html#class-json-method-parse-string
static func read(json_path: String) -> Dictionary:
	var s := FileAccess.get_file_as_string(json_path)
	if s == "":
		push_error("UeMaterial: cannot read " + json_path)
		return {}
	var d = JSON.parse_string(s)
	return d if d is Dictionary else {}

# "Mordhau/Content/.../theFallen_rma.0" -> res://data/Mordhau/Content/.../theFallen_rma.png
static func tex_path(obj_path: String) -> String:
	var p := obj_path.get_basename() if obj_path.get_extension().is_valid_int() else obj_path
	return DATA + p + ".png"

static func _obj_pkg(obj_path: String) -> String:
	return obj_path.get_basename() if obj_path.get_extension().is_valid_int() else obj_path

# Texture reference -> content path. A texture that is a sub-export of another package (HLOD proxies: ObjectPath
# "…/HLOD/Arena_0_HLOD.213", ObjectName "Texture2D'T_Arena_0_HLOD_0_…_Diffuse'") is written by the exporter as
# <package>/<object name>.png, so the object's own name is appended when it differs from the package name.
static func _tex_obj(v) -> String:
	if not (v is Dictionary):
		return ""
	var op := String(v.get("ObjectPath", ""))
	var on := String(v.get("ObjectName", ""))
	var nm := on.get_slice("'", 1) if on.contains("'") else on
	var pkg := _obj_pkg(op)
	if op == "" or nm == "" or pkg.get_file() == nm:
		return op
	return pkg + "/" + nm

static func _vec(c) -> Vector3:
	return Vector3(c.get("R", 1.0), c.get("G", 1.0), c.get("B", 1.0)) if c is Dictionary else Vector3.ONE

# Merged parameters, nearest first. Returns {tex: [[slot, obj_path]...], colors: {}, scalars: {}, blend, clip, two_sided,
# chain: [package names]}.
static func params(json_path: String) -> Dictionary:
	var out := {"tex": [], "colors": {}, "scalars": {}, "switches": {}, "blend": -1, "clip": -1.0, "two_sided": null,
		"foliage": null, "chain": [], "master": ""}
	var d := read(json_path)
	if d.is_empty():
		return {}
	for k in d.get("Textures", {}):
		if d.Textures[k] is Dictionary:
			out.tex.append([k, _tex_obj(d.Textures[k])])
	for k in d.get("Colors", {}):
		out.colors[k] = _vec(d.Colors[k])
	for k in d.get("Scalars", {}):
		out.scalars[k] = float(d.Scalars[k])
	for k in d.get("Switches", {}):
		out.switches[k] = bool(d.Switches[k])
	var pkg := json_path.trim_prefix(DATA).trim_suffix(".json")
	var own := pkg
	# A material that is a sub-export of another package (HLOD proxies: …/HLOD/Arena_0_HLOD/M_….json from the exporter,
	# package …/HLOD/Arena_0_HLOD) is looked up by name inside that package.
	# Map-embedded materials (…/Contraband/Contraband/PersistentLevel/M_….json) sit deeper: walk up to the package.
	var sub := ""
	if not FileAccess.file_exists(UePkg.root() + "/json/" + pkg + ".json"):
		var up := pkg.get_base_dir()
		while up.contains("/") and not FileAccess.file_exists(UePkg.root() + "/json/" + up + ".json"):
			up = up.get_base_dir()
		if FileAccess.file_exists(UePkg.root() + "/json/" + up + ".json"):
			sub = pkg.get_file()
			pkg = up
		else:
			pkg = ""   # no package json: own-file parameters only
	for i in 16:
		var exports := UePkg.load_pkg(pkg) if pkg != "" else []
		var e := {}
		for x in exports:
			if String(x.get("Type", "")).begins_with("Material") and (sub == "" or x.get("Name", "") == sub):
				e = x; break
		sub = ""
		if e.is_empty():
			break
		out.chain.append(e.get("Name", ""))
		var p: Dictionary = e.get("Properties", {})
		if e.Type == "Material":
			out.master = e.get("Name", "")
			_master(p, out)
			if pkg != own and not FileAccess.file_exists(DATA + pkg + ".json"):
				# Master not exported by mdx (exportlist exports the instances a map lists): its export's "Textures" are
				# CachedExpressionData.ReferencedTextures keyed by texture name (CUE4Parse UMaterial.cs:46-47 load,
				# :272-275 GetParams `parameters.Textures[texture.Name] = texture`), so read the same list from the package.
				for t in p.get("CachedExpressionData", {}).get("ReferencedTextures", []):
					var on := String(t.get("ObjectName", "")) if t is Dictionary else ""
					if on != "":
						out.tex.append([on.get_slice("'", 1) if on.contains("'") else on, _tex_obj(t)])
			else:
				_gltf_textures(pkg, own, out)
			break
		for t in p.get("TextureParameterValues", []):
			if t.get("ParameterValue") is Dictionary:
				out.tex.append([t.ParameterInfo.Name, _tex_obj(t.ParameterValue)])
		for v in p.get("VectorParameterValues", []):
			if not out.colors.has(v.ParameterInfo.Name):
				out.colors[v.ParameterInfo.Name] = _vec(v.ParameterValue)
		for v in p.get("ScalarParameterValues", []):
			if not out.scalars.has(v.ParameterInfo.Name):
				out.scalars[v.ParameterInfo.Name] = float(v.ParameterValue)
		for v in p.get("StaticParameters", {}).get("StaticSwitchParameters", []):
			if not out.switches.has(v.ParameterInfo.Name):
				out.switches[v.ParameterInfo.Name] = bool(v.Value)
		_gltf_textures(pkg, own, out)
		# An instance's override counts only with its bOverride_ flag; without it the value is a copy of the parent's, so
		# the walk continues to the parent (UE "Material Instances" docs, Material Property Overrides:
		# https://docs.unrealengine.com/4.26/en-US/RenderingAndGraphics/Materials/MaterialInstances/).
		var o: Dictionary = p.get("BasePropertyOverrides", {})
		if out.blend < 0 and o.get("bOverride_BlendMode", false):
			out.blend = BLEND.get(o.get("BlendMode", "BLEND_Opaque"), 0)
		if out.clip < 0 and o.get("bOverride_OpacityMaskClipValue", false):
			out.clip = float(o.get("OpacityMaskClipValue", 0.3333))
		if out.two_sided == null and o.get("bOverride_TwoSided", false):
			out.two_sided = bool(o.get("TwoSided", false))
		if out.foliage == null and o.get("bOverride_ShadingModel", false):
			out.foliage = o.get("ShadingModel", "") == "MSM_TwoSidedFoliage"
		pkg = _obj_pkg(p.get("Parent", {}).get("ObjectPath", ""))
	if out.blend < 0:
		out.blend = int(d.get("BlendMode", 0))
	if out.clip < 0:
		out.clip = float(d.get("Properties", {}).get("BasePropertyOverrides", {}).get("OpacityMaskClipValue", 0.3333))
	if out.two_sided == null:
		out.two_sided = false
	if out.foliage == null:
		out.foliage = false
	return out

# Textures a parent or master samples directly (no parameter) are only in its own mdx export, res://data/<pkg>.json
# (e.g. RomanKit_Background samples romankit_background_basecolor; MetalWood_Master WdMtlRpe_Trim_basecolor/_clrmask).
static func _gltf_textures(pkg: String, own: String, out: Dictionary) -> void:
	if pkg == own or not FileAccess.file_exists(DATA + pkg + ".json"):
		return
	var g := read(DATA + pkg + ".json")
	for k in g.get("Textures", {}):
		if g.Textures[k] is Dictionary:
			out.tex.append([k, _tex_obj(g.Textures[k])])

static func _master(p: Dictionary, out: Dictionary) -> void:
	var cp: Dictionary = p.get("CachedExpressionData", {}).get("Parameters", {})
	var names := func(k: String) -> Array:
		return cp.get(k, {}).get("ParameterInfos", []).map(func(i): return i.Name)
	var tn: Array = names.call("RuntimeEntries[2]")
	var tv: Array = cp.get("TextureValues", [])
	for i in mini(tn.size(), tv.size()):
		if tv[i] is Dictionary:
			out.tex.append([tn[i], _tex_obj(tv[i])])
	var vn: Array = names.call("RuntimeEntries[1]")
	var vv: Array = cp.get("VectorValues", [])
	for i in mini(vn.size(), vv.size()):
		if not out.colors.has(vn[i]):
			out.colors[vn[i]] = _vec(vv[i])
	var sn: Array = names.call("RuntimeEntries")
	var sv: Array = cp.get("ScalarValues", [])
	for i in mini(sn.size(), sv.size()):
		if not out.scalars.has(sn[i]):
			out.scalars[sn[i]] = float(sv[i])
	if out.blend < 0:
		out.blend = BLEND.get(p.get("BlendMode", "BLEND_Opaque"), 0)
	if out.clip < 0 and p.has("OpacityMaskClipValue"):
		out.clip = float(p.OpacityMaskClipValue)
	if out.two_sided == null and p.has("TwoSided"):
		out.two_sided = bool(p.TwoSided)
	if out.foliage == null and p.has("ShadingModel"):
		out.foliage = p.ShadingModel == "MSM_TwoSidedFoliage"

# Role of a slot that is a texture name ("WoodWall_A" -> "A"), or "".
static func suffix_role(name: String) -> String:
	var n := name.to_lower()
	for s in SUFFIX:
		if n.ends_with(s[0]):
			return s[1]
	return ""

# Picks {albedo, normal, packed, layout} as res:// png paths from params().tex.
# ResourceLoader.exists: https://docs.godotengine.org/en/stable/classes/class_resourceloader.html
static func pick(tex: Array) -> Dictionary:
	var have := []
	for t in tex:
		var p := tex_path(t[1])
		if t[1] != "" and ResourceLoader.exists(p):
			have.append([t[0], p, t[1].get_file().get_basename()])
	var first := func(names: Array) -> Array:
		for n in names:
			for h in have:
				if h[0] == n:
					return h
		return []
	var by_suffix := func(role: String) -> Array:
		for h in have:
			# Engine placeholders (Default_d, DefaultDiffuse) are never the real texture.
			if h[0] == h[2] and suffix_role(h[0]) == role and not h[0].to_lower().begins_with("default"):
				return h
		return []
	var out := {"albedo": "", "normal": "", "packed": "", "layout": "", "albedo_missing": "", "clrmask": "", "layers": []}
	# Directly sampled color mask (MetalWood_Master: WdMtlRpe_Trim_clrmask), channels R/G/B -> Color_R/G/B.
	for h in have:
		if h[0] == h[2] and h[0].to_lower().ends_with("_clrmask"):
			out.clrmask = h[1]; break
	# Extra layers 2..4: BaseColor_0n + Normal_0n + RHAO_0n (Castle_Mat_Optimized_ItBump_Arena*, Arena_Blend_M).
	for i in range(2, 5):
		var la: Array = first.call(["BaseColor_0%d" % i])
		if la.is_empty():
			break
		var ln: Array = first.call(["Normal_0%d" % i])
		var lp: Array = first.call(["RHAO_0%d" % i])
		out.layers.append([la[1], ln[1] if ln else "", lp[1] if lp else ""])
	# A named albedo slot whose texture was not exported (master default such as Cloth_M's shipsail_basecolor).
	for nm in SLOT_ALBEDO:
		for t in tex:
			if t[0] == nm and t[1] != "" and not ResourceLoader.exists(tex_path(t[1])) and out.albedo_missing == "":
				out.albedo_missing = t[1]
	var a: Array = first.call(SLOT_ALBEDO)
	if a.is_empty(): a = by_suffix.call("A")
	if a.is_empty():
		for h in have:
			if h[0] == h[2] and suffix_role(h[0]) == "" and not NOT_ALBEDO.any(func(x): return h[0].to_lower().contains(x)):
				a = h; break
	if a.is_empty():
		a = first.call(["PM_Diffuse"])
		if a and suffix_role(a[2]) not in ["", "A"]:
			a = []
	out.albedo = a[1] if a else ""
	var n: Array = first.call(SLOT_NORMAL)
	if n.is_empty(): n = by_suffix.call("N")
	if n.is_empty(): n = first.call(["PM_Normals"])
	out.normal = n[1] if n else ""
	for slot in SLOT_PACKED:
		var h: Array = first.call([slot])
		if h:
			out.packed = h[1]; out.layout = SLOT_PACKED[slot]
			# A "Roughness Texture" that is really an RMA (prop_barrel_01_r) still has roughness in R.
			break
	if out.packed == "":
		# "_r" (MaleHead01_r, prop_barrel_01_r) is the weakest hint: a master's default list can hold another asset's _r
		# (M_CY_RoundBanner lists T_HeavyTabard_r next to its own T_CY_RoundBanner_RHAO).
		# Masters also list other layers' packed textures (MetalWood_Master: Charcoal_RHAO for its burn layer), so the
		# candidate sharing the longest name prefix with the chosen albedo wins (WdMtlRpe_Trim_RMAO for
		# WdMtlRpe_Trim_basecolor); ties go by the role order.
		var roles := ["RHAO", "SRMH", "RMA", "MRA", "RMA_R"]
		var alb_name: String = a[2] if a else ""
		var best := []
		var best_score := -1
		for h in have:
			var role := suffix_role(h[0])
			if h[0] != h[2] or role not in roles or h[0].to_lower().begins_with("default"):
				continue
			var pre := 0
			while pre < mini(alb_name.length(), h[2].length()) and alb_name[pre].to_lower() == h[2][pre].to_lower():
				pre += 1
			var score: int = pre * 10 + (4 - roles.find(role))
			if score > best_score:
				best_score = score
				best = [h[1], role]
		if best:
			out.packed = best[0]; out.layout = "RMA" if best[1] == "RMA_R" else best[1]
	if out.packed == "":
		var h: Array = first.call(["PM_SpecularMasks"])
		var role := suffix_role(h[2]) if h else ""
		if h and role in ["RMA", "RHAO", "SRMH", "MRA", "RMA_R"]:
			out.packed = h[1]; out.layout = "RMA" if role == "RMA_R" else role
	return out

# Shader mode per master: the masters whose compiled shaders are read in docs/SHADERS.md get their own code path.
# Instances with other static-switch sets compile their own permutation (SHADERS.md 11); those used here and not dumped
# are noted in MODE_NOTES.
const MODES := {
	"M_WeaponMaster": "M_WEAPON",                                  # SHADERS.md 5
	"MetalWood_Master": "M_METALWOOD",                             # SHADERS.md 5 (master-default permutation)
	"Castle_Mat_Optimized_ItBump_Arena_New2": "M_LAYERED",         # SHADERS.md 6a
	"Castle_Mat_Optimized_ItBump_Arena": "M_LAYERED",              # same parameter set; not dumped
	"Arena_Blend_M": "M_ARENABLEND",                               # SHADERS.md 6b
	"Architecture_M_New": "M_ARCH",                                # SHADERS.md 6c (RedPaint permutation)
	"RomanKit_Background": "M_ROMANKITBG",                         # SHADERS.md 6d
	"M_CY_RoundBanner": "M_ROUNDBANNER",                           # SHADERS.md 8
	# Same compiled code as 6d (RomanTrimBackground_01/r2/006 lines 4-30: albedo saturate(tex0), roughness
	# 1 - albedo.r^2.5, no AO / grunge / MultiplyAdjust; state/shaders, extract.sh round 6).
	"RomanTrim_Background": "M_ROMANKITBG",
	"M_Bush": "M_BUSH",                                            # state/shaders/M_Bush/r0/009 (ue_tint.gdshader)
	"M_MegascansFoliageMaster": "M_MEGAFOLIAGE",                   # INST_SparseGrass_Arena/r2/009 (ue_tint.gdshader)
	"M_MordhauHLOD": "M_HLOD",                                     # M_MordhauHLOD/r0/053 (ue_tint.gdshader)
	"Cloth_M": "M_CLOTH",                                          # Cloth_M/r0/083 (ue_tint.gdshader)
	"Tent_Cloth_Master": "M_CLOTH",                                # Tent_Cloth_Castello_Red_01/r0 (ue_tint.gdshader)
	"BannerAtlasTweak_M": "M_BANNER",                              # INST_BannerRed_WPO/r0 (ue_tint.gdshader)
	"Grass_mat1_Castello": "M_GRASSCAST",                          # Grass_mat1_Castello/r0/009 (ue_tint.gdshader)
	"M_PLains_Fern01": "M_FERN",                                   # M_PLains_Fern01/r0/009 (ue_tint.gdshader)
	"Flora_twoSides_ALT": "M_FLORA",                               # Flora_twoSides_ALT/r1/009 (ue_tint.gdshader)
	"Treasure_Decal_Ceramics": "M_DECALDIRT",                      # Treasure_Decal_Ceramics/r0/009 (ue_tint.gdshader)
}
const MODE_WEARABLE := "M_WEARABLE"                                # SHADERS.md 7, used by game/character/character_builder.gd

# Shader for a render state: ue_tint.gdshader (opaque, one-sided) or one of the ue_tint_<masked|translucent>[_2s].gdshader
# files, which #define UE_MASKED / UE_TRANSLUCENT / UE_TWO_SIDED and include ue_tint.gdshaderinc. Loaded by path, so every
# material (imported or built at runtime) shares these few Shader resources; one Shader = one set of Vulkan pipelines
# (round 12). foliage / lightmap / mode / vlm no longer pick a shader: set_variant() sets them as uniforms.
# Shader.code: https://docs.godotengine.org/en/stable/classes/class_shader.html
static func shader_for(blend: int, two_sided: bool, _foliage := false, _lightmap := false, _mode := "", _vlm := false) -> Shader:
	var name := "ue_tint"
	if blend == 1: name += "_masked"
	elif blend == 2: name += "_translucent"
	if two_sided:
		name += "_2s"
	return load(SHADER.get_base_dir().path_join(name + ".gdshader"))

# ue_mode uniform values (ue_tint.gdshaderinc, the ue_mode block).
const MODE_IDS := {"": 0, "M_WEAPON": 1, "M_METALWOOD": 2, "M_LAYERED": 3, "M_ARENABLEND": 4, "M_ARCH": 5, "M_ROMANKITBG": 6,
	"M_WEARABLE": 7, "M_ROUNDBANNER": 8, "M_CLOTH": 9, "M_BANNER": 10, "M_GRASSCAST": 11, "M_FERN": 12, "M_FLORA": 13,
	"M_DECALDIRT": 14, "M_HLOD": 15, "M_MEGAFOLIAGE": 16, "M_BUSH": 17}

# The material's variant: render-state shader plus the master mode and lighting flags as uniforms.
# vlm: volumetric-lightmap indirect light for geometry with no baked lightmap; never together with the lightmap.
static func set_variant(m: ShaderMaterial, blend: int, two_sided: bool, foliage := false, lightmap := false, mode := "", vlm := false) -> void:
	m.shader = shader_for(blend, two_sided)
	m.set_shader_parameter("ue_mode", MODE_IDS.get(mode, 0))
	m.set_shader_parameter("use_foliage", foliage)
	m.set_shader_parameter("use_lightmap", lightmap)
	m.set_shader_parameter("use_vlm", vlm and not lightmap)
	m.set_meta("ue_variant", [blend, two_sided, foliage, mode])

# True if at least 1% of the texels pass `clip` in the png's alpha (sampled every 4th texel).
# Image.load_png_from_buffer / get_pixel: https://docs.godotengine.org/en/stable/classes/class_image.html
static func alpha_is_mask(png: String, clip: float) -> bool:
	var key := "%s@%s" % [png, clip]
	if _alpha_ok.has(key):
		return _alpha_ok[key]
	var img := Image.new()
	var ok := false
	if img.load_png_from_buffer(FileAccess.get_file_as_bytes(png)) == OK and img.detect_alpha() != Image.ALPHA_NONE:
		var n := 0; var pass_ := 0
		for y in range(0, img.get_height(), 4):
			for x in range(0, img.get_width(), 4):
				n += 1
				pass_ += int(img.get_pixel(x, y).a >= clip)
		ok = n > 0 and float(pass_) / n >= 0.01
	_alpha_ok[key] = ok
	return ok

# First exported texture for slot name `slot` (nearest in the chain), "" if none.
static func slot_png(p: Dictionary, slot: String) -> String:
	for t in p.tex:
		if t[0] == slot and t[1] != "" and ResourceLoader.exists(tex_path(t[1])):
			return tex_path(t[1])
	return ""

# The texture's UTexture SRGB flag from its package json (mdx json omits it when true, the UTexture default).
static func tex_srgb(png: String) -> bool:
	for e in UePkg.load_pkg(png.trim_prefix(DATA).trim_suffix(".png")):
		if String(e.get("Type", "")).begins_with("Texture"):
			return bool(e.get("Properties", {}).get("SRGB", true))
	return true

# ShaderMaterial.set_shader_parameter: https://docs.godotengine.org/en/stable/classes/class_shadermaterial.html
static func build(material_json_path: String) -> Material:
	var p := params(material_json_path)
	if p.is_empty():
		return null
	var k := pick(p.tex)
	var mode: String = MODES.get(p.master, "")
	var m := ShaderMaterial.new()
	# Map materials get the UE_VLM path (unlit by UE's per-component lightmap until with_lightmap swaps it out); it stays
	# inert outside a level (vlm_scale 0). Wearables (wearable()) and character builds do not use build().
	set_variant(m, p.blend, p.two_sided, p.foliage, false, mode, true)
	m.resource_name = material_json_path.get_file().get_basename()
	# Runtime rebuild key (rebuild_all): the material JSON this was built from, and the shader source it was built with.
	m.set_meta("ue_json", material_json_path)
	m.set_meta("ue_shader_rev", shader_rev())
	m.set_meta("ue_blend", p.blend)
	m.set_meta("ue_mode", mode)
	m.set_meta("ue_layout", k.layout)
	if k.albedo != "":
		m.set_shader_parameter("albedo_texture", load(k.albedo))
	if k.normal != "":
		m.set_shader_parameter("normal_texture", load(k.normal))
	m.set_shader_parameter("use_normal", k.normal != "")
	if k.packed != "":
		m.set_shader_parameter("rma_texture", load(k.packed))
	var lay: Array = LAYOUTS[k.layout]
	m.set_shader_parameter("rough_sel", lay[0])
	m.set_shader_parameter("metal_sel", lay[1])
	m.set_shader_parameter("ao_sel", lay[2])
	m.set_shader_parameter("rough_add", p.scalars.get("Roughness", lay[3]) if k.packed == "" else lay[3])
	m.set_shader_parameter("metal_add", lay[4])
	m.set_shader_parameter("ao_add", lay[5])
	if p.blend in [1, 2]:
		m.set_shader_parameter("alpha_clip", p.clip)
		m.set_shader_parameter("alpha_from_albedo", k.albedo != "" and alpha_is_mask(k.albedo, p.clip if p.blend == 1 else 0.01))
		m.set_shader_parameter("alpha_value", 0.0 if p.blend == 2 and k.albedo == "" else 1.0)
	var S := func(name: String, key: String, def: float) -> void:
		m.set_shader_parameter(name, p.scalars.get(key, def))
	var V := func(name: String, key: String, def: Vector3) -> void:
		m.set_shader_parameter(name, p.colors.get(key, def))
	match mode:
		"M_WEAPON":
			var cm := slot_png(p, "ColorMap")
			if cm != "":
				m.set_shader_parameter("color_mask", load(cm))
			for c in ["A", "B", "C"]:
				V.call("color_" + c.to_lower(), "Color" + c, Vector3.ONE)
			S.call("metal_tweak", "Metal Tweak", 1.15)
			S.call("roughness_power", "RoughnessPower", 1.0)
			S.call("ao_weaken", "AOWeaken", 0.0)
		"M_METALWOOD":
			if k.clrmask != "":
				m.set_shader_parameter("color_mask", load(k.clrmask))
			V.call("color_var", "ColorVar", Vector3(0.5, 0.5, 0.5))
			V.call("color_overlay", "ColorOverlay", Vector3(0.5, 0.5, 0.5))
			S.call("color_desat", "ColorDesaturaton", 0.0)
			S.call("ao_bias", "AOBias", 0.0)
			S.call("metal_contrast", "Metal_Contrast", 0.0)
			V.call("metal_color_lerp", "MetalColorLerp", Vector3(0.5, 0.5, 0.5))
			S.call("metal_color_bias", "MetalColorBias", 0.0)
			S.call("rough_lerp_wood", "RoughnessLerp_Wood", 1.0)
			S.call("rough_lerp_wood_bias", "RoughnessLerpWood_BIas", 0.0)
			S.call("metal_roughness", "MetalRoughness", 0.0)
		"M_LAYERED", "M_ARENABLEND":
			_layers(m, p, k)
			if mode == "M_LAYERED":
				S.call("ao_bias", "AO_Bias", 0.0)
				var gr := slot_png(p, "GrungeTexture")
				m.set_shader_parameter("use_grunge", p.switches.get("UseGrunge", false) and p.switches.get("UseGrungeB", false) and gr != "")
				if gr != "":
					m.set_shader_parameter("grunge_texture", load(gr))
					m.set_shader_parameter("grunge_srgb", tex_srgb(gr))
				S.call("grunge_scale", "Scale", 1.0)
				S.call("contrast_b", "Contrast_B", 1.0)
				V.call("grunge_color_b", "Color_B", Vector3.ONE)
				S.call("overall_bias", "OverallBias", 0.0)
				S.call("metallic_stain", "Metallic_Stain", 0.0)
				S.call("roughness_stain", "Roughness_Stain", 0.9)
				S.call("spec_04", "Spec_04", 0.5)
			else:
				_paint(m, p)
				S.call("paint_flatten_normal", "PaintFlattenNormal", 0.5)
		"M_ARCH":
			_paint(m, p)
			var d := slot_png(p, "RHAODetail")
			if d != "": m.set_shader_parameter("rhao_detail", load(d))
			var nd := slot_png(p, "NormalDetail")
			m.set_shader_parameter("use_normal_detail", nd != "" and k.normal != "")
			if nd != "": m.set_shader_parameter("normal_detail", load(nd))
			var ns: Vector3 = p.colors.get("NonTriplanarNormalUVScale", Vector3.ONE)
			m.set_shader_parameter("normal_detail_scale", Vector2(ns.x, ns.y))
			var ad := slot_png(p, "AlbedoDetail")
			if ad != "": m.set_shader_parameter("albedo_detail", load(ad))
			var rs: Vector3 = p.colors.get("NonTriplanarRHAOUVScale", Vector3.ONE)
			var cs: Vector3 = p.colors.get("NonTriplanarColorUVScale", Vector3.ONE)
			m.set_shader_parameter("rhao_detail_scale", Vector2(rs.x, rs.y))
			m.set_shader_parameter("albedo_detail_scale", Vector2(cs.x, cs.y))
			S.call("a_desaturation", "Desaturation", 0.0)
			V.call("a_color", "Color", Vector3.ONE)
			S.call("detail_power", "Detail_Power", 1.0)
			S.call("desaturation_detail", "Desaturation_Detail", 0.0)
			V.call("albedo_color", "AlbedoColor", Vector3.ONE)
			S.call("overlay_bias", "Bias", 0.0)
			V.call("curvature_color", "CurvatureColor", Vector3.ONE)
			V.call("dirt_color", "DirtColor", Vector3(0.5, 0.5, 0.5))
			S.call("dirt_power", "DirtPower", 3.0)
			S.call("dirt_contrast", "Dirt_Contrast_01", 0.4)
			S.call("damage_desaturation", "Damage_Desaturation", 0.0)
			V.call("color_damage", "ColorDamage", Vector3(0.5, 0.5, 0.5))
			S.call("roughness_bias", "RoughnessBias", 0.5)
			S.call("ao_bias", "AO_Bias", 1.0)
			S.call("bias_normal", "Bias_Normal", 0.0)
		"M_ROUNDBANNER":
			for t in p.tex:
				if t[0] == t[1].get_file().get_basename() and t[0].to_lower().ends_with("_c") and ResourceLoader.exists(tex_path(t[1])):
					m.set_shader_parameter("color_mask", load(tex_path(t[1])))
					break
			V.call("color_a", "ColorA", Vector3.ONE)
			V.call("color_b", "ColorB", Vector3.ONE)
			S.call("bleed", "Bleed", 1.5)
			S.call("albedo_scale", "Albedo", 0.75)
			S.call("roughness_scale", "Roughness", 1.0)
			var em := slot_png(p, "EmblemTexture")
			if em != "": m.set_shader_parameter("emblem_texture", load(em))
			V.call("emblem_color_a", "EmblemColorA", Vector3.ONE)
			V.call("emblem_color_b", "EmblemColorB", Vector3(0, 0, 1))
			V.call("sss", "SSS", Vector3.ONE)
		"M_CLOTH":
			var tent: bool = p.master == "Tent_Cloth_Master"
			var al := slot_png(p, "Albedo")
			if al != "": m.set_shader_parameter("albedo_texture", load(al))
			var g := func(k: String, d: float) -> float: return p.scalars.get(k, d)
			m.set_shader_parameter("cloth_tile", Vector2(g.call("Tile_Amount_02", 1.0), g.call("TileAmount_Detail" if tent else "TileAmount", 1.0)))
			S.call("cloth_desat", "Desaturation", 0.0)
			V.call("cloth_mul", "ColorMultiply", Vector3.ONE)
			for t in [["detail_albedo", "linencloth_basecolor"], ["detail_normal", "linencloth_normal"]]:
				var tp := slot_png(p, t[1])
				if tp != "": m.set_shader_parameter(t[0], load(tp))
			var gp := slot_png(p, "GrungeTexture" if tent else "Grunge_Tex")
			if gp != "":
				m.set_shader_parameter("cloth_grunge", load(gp))
				m.set_shader_parameter("cloth_grunge_srgb", tex_srgb(gp))
			m.set_shader_parameter("cloth_grunge_uv1", not tent)
			S.call("cloth_grunge_scale", "Scale_R" if tent else "Grunge_Scale", 1.0)
			m.set_shader_parameter("cloth_grunge_contrast", g.call("Contrast_R", 0.0) if tent else 0.0)
			V.call("cloth_grunge_color", "Color_R" if tent else "Grunge_Color", Vector3.ONE)
			m.set_shader_parameter("cloth_grunge_bias", g.call("OverallBias", 0.0) if tent else 1.0)
			S.call("cloth_nm_bias", "NM_Strength_Detail" if tent else "NM_Strength_Bias", 0.0)
			var rp := slot_png(p, "RMA" if tent else "RHAO")
			if rp != "": m.set_shader_parameter("rma_texture", load(rp))
		"M_BANNER":
			var g := func(k: String, d: float) -> float: return p.scalars.get(k, d)
			for t in [["detail_albedo", "linencloth_basecolor"], ["rma_texture", "Banner_RMA"], ["normal_texture", "Banner_Normal"]]:
				var tp := slot_png(p, t[1])
				if tp != "": m.set_shader_parameter(t[0], load(tp))
			m.set_shader_parameter("use_normal", slot_png(p, "Banner_Normal") != "")
			var bp := slot_png(p, "BannerID")
			if bp != "":
				m.set_shader_parameter("banner_mask", load(bp))
				m.set_shader_parameter("banner_mask_srgb", tex_srgb(bp))
			var gp := slot_png(p, "GrungeTexture")
			if gp != "":
				m.set_shader_parameter("cloth_grunge", load(gp))
				m.set_shader_parameter("cloth_grunge_srgb", tex_srgb(gp))
			m.set_shader_parameter("banner_detail_scale", Vector2(g.call("U_Scale_Detail", 5.0), g.call("V_Scale_Detail", 5.0)))
			V.call("color_a", "ColorA", Vector3(1, 0, 0))
			V.call("color_b", "ColorB", Vector3(0, 0, 1))
			S.call("bleed", "Bleed", 1.5)
			S.call("banner_scale_r", "Scale_R", 0.0)
			S.call("banner_contrast_r", "Contrast_R", 1.0)
			V.call("banner_color_r", "Color_R", Vector3.ONE)
			S.call("banner_scale_b", "Scale_B", 0.0)
			S.call("banner_contrast_b", "Contrast_B", 1.0)
			V.call("banner_color_b", "Color_B", Vector3.ONE)
			S.call("banner_bias", "OverallBias", 0.0)
			S.call("banner_sss", "SSS", 0.3)
		"M_GRASSCAST", "M_FERN":
			var gd := slot_png(p, "Grass_01" if mode == "M_GRASSCAST" else "T_Plains_Fern01_D")
			if gd != "": m.set_shader_parameter("albedo_texture", load(gd))
			if mode == "M_FERN":
				var fn := slot_png(p, "T_Plains_Fern01_N")
				if fn != "": m.set_shader_parameter("normal_texture", load(fn))
				m.set_shader_parameter("use_normal", fn != "")
		"M_FLORA":
			S.call("flora_tiling", "TextureTiling", 1.0)
			V.call("flora_color", "Color", Vector3.ONE)
			S.call("flora_color_lerp", "ColorLerp", 0.0)
			S.call("flora_desat", "desaturation", 0.0)
			S.call("flora_hsl", "HSL", 0.0)
			V.call("flora_mul", "ColorMultiply", Vector3.ONE)
			S.call("flora_emissive", "Emissive", 0.0)
			V.call("flora_sub", "SubColor", Vector3.ONE)
			S.call("flora_ss_bias", "SS_Bias", 0.0)
			S.call("flora_metallic", "metallic", 0.0)
			S.call("flora_spec", "spec", 0.0)
			S.call("flora_rough", "Roughness", 0.0)
			S.call("flora_nm_flat", "NM_Flatness", 0.0)
		"M_DECALDIRT":
			var dm := slot_png(p, "treasuretrims_CrvMaskDirt")
			if dm != "": m.set_shader_parameter("color_mask", load(dm))
		"M_HLOD":
			var dt := slot_png(p, "DiffuseTexture")
			m.set_shader_parameter("use_diffuse_tex", p.switches.get("UseDiffuse", false) and dt != "")
			if dt != "": m.set_shader_parameter("albedo_texture", load(dt))
			var nt := slot_png(p, "NormalTexture")
			if nt != "":
				m.set_shader_parameter("normal_texture", load(nt))
				m.set_shader_parameter("use_normal", p.switches.get("UseNormal", true))
			V.call("diffuse_const", "DiffuseConst", Vector3.ZERO)   # master defaults (M_MordhauHLOD.json)
			S.call("metallic_const", "MetallicConst", 0.0)
			S.call("specular_const", "SpecularConst", 0.0)
			S.call("roughness_const", "RoughnessConst", 0.9)
			S.call("ao_const", "AmbientOcclusionConst", 1.0)
		"M_MEGAFOLIAGE":
			V.call("albedo_tint", "Albedo", Vector3.ONE)
			S.call("mf_desaturation", "Desaturation", 0.0)
			S.call("hue_shift", "Hue Shift", 0.0)
			S.call("mf_emissive", "Emissive", 0.0)
			S.call("top_power", "TopPower", 0.5)
			S.call("top_darken", "TopDarken", 0.5)
			S.call("mf_roughness", "Roughness", 1.0)
			V.call("mf_sss", "SSS", Vector3.ONE)
			S.call("mf_specular", "SpecMax", 0.2)
			var tr := slot_png(p, "Translucency Texture")
			if tr != "":
				m.set_shader_parameter("translucency_texture", load(tr))
				m.set_shader_parameter("translucency_srgb", tex_srgb(tr))
		"M_BUSH":
			var vm := slot_png(p, "T_Foliage_Variation_Mask")
			if vm != "":
				m.set_shader_parameter("variation_texture", load(vm))
				m.set_shader_parameter("variation_srgb", tex_srgb(vm))
			S.call("saturation", "Saturation", 0.0)
			V.call("bush_color", "Color", Vector3.ONE)
			S.call("brightness", "Brightness", 1.0)
			S.call("variation_tiling", "Variation_Tiling", 5000.0)
			S.call("variation_intensity", "Variation_Intensity", 0.1)
			S.call("ao_power", "AO", 1.0)
			S.call("emissive", "Emissive", 0.0)
			S.call("roughness_power", "Roughness", 2.0)
			S.call("specular_value", "Specular", 0.15)
	# No albedo texture: a flat color from the first of BASE_COLORS the material sets.
	if k.albedo == "":
		var base = null
		for c in BASE_COLORS:
			if p.colors.has(c):
				base = p.colors[c]; break
		m.set_shader_parameter("base_color", base if base != null else Vector3.ONE)
		m.set_meta("ue_flat", base != null or mode == "M_DECALDIRT")
		if k.albedo_missing != "":
			m.set_meta("ue_missing_albedo", k.albedo_missing)
	return m

# Wearable material (SHADERS.md 7, M_EquipmentMasterChestShoulder): the runtime inputs UE sets on the dynamic instance.
# `primary` / `secondary` = {albedo, normal, rma, mask: Texture2D or null, colors: [ColorA, ColorB, ColorC] as Vector3,
# has_emblem: bool}; secondary {} = none (every vertex then uses the primary set). Emblem: {texture, color_a, color_b} or {}.
# Mask channels: R -> ColorA, B -> ColorB, G -> ColorC (059 lines 94-103), each multiplying the untinted albedo.
static func wearable(primary: Dictionary, secondary := {}, emblem := {}, masked := false) -> ShaderMaterial:
	var m := ShaderMaterial.new()
	set_variant(m, 1 if masked else 0, false, false, false, MODE_WEARABLE)
	m.set_meta("ue_mode", MODE_WEARABLE)
	var keys := {"albedo": "albedo_texture", "normal": "normal_texture", "rma": "rma_texture", "mask": "color_mask"}
	for k in keys:
		if primary.get(k):
			m.set_shader_parameter(keys[k], primary[k])
	m.set_shader_parameter("use_normal", primary.get("normal") != null)
	var pc: Array = primary.get("colors", [Vector3(1, 0, 0), Vector3.ZERO, Vector3.ONE])
	for i in 3:
		m.set_shader_parameter(["color_a", "color_b", "color_c"][i], pc[i])
	m.set_shader_parameter("primary_has_emblem", 1.0 if primary.get("has_emblem", false) else 0.0)
	if not secondary.is_empty():
		m.set_shader_parameter("has_secondary", true)
		var sk := {"albedo": "secondary_albedo", "normal": "secondary_normal", "rma": "secondary_rma", "mask": "secondary_mask"}
		for k in sk:
			if secondary.get(k):
				m.set_shader_parameter(sk[k], secondary[k])
		var sc: Array = secondary.get("colors", [Vector3(1, 0, 0), Vector3.ZERO, Vector3.ONE])
		for i in 3:
			m.set_shader_parameter(["secondary_color_a", "secondary_color_b", "secondary_color_c"][i], sc[i])
		m.set_shader_parameter("secondary_has_emblem", 1.0 if secondary.get("has_emblem", false) else 0.0)
	if emblem.get("texture"):
		m.set_shader_parameter("emblem_texture", emblem.texture)
		m.set_shader_parameter("emblem_color_a", emblem.get("color_a", Vector3.ONE))
		m.set_shader_parameter("emblem_color_b", emblem.get("color_b", Vector3(0, 0, 1)))
	return m

# Layer textures and blend parameters (SHADERS.md 6a/6b): layer n = BaseColor/Normal/RHAO_0n while UseLayerN is on;
# vertex R/G/B weight layers 2/3/4 through the height-lerp in ue_tint.gdshader.
static func _layers(m: ShaderMaterial, p: Dictionary, k: Dictionary) -> void:
	var g := func(key: String, def: float) -> float: return p.scalars.get(key, def)
	m.set_shader_parameter("tiling", Vector4(g.call("Tiling_01", 1.0), g.call("Tiling_02", 1.0), g.call("Tiling_03", 1.0), g.call("Tiling_04", 1.0)))
	m.set_shader_parameter("height_contrast", Vector4(g.call("HeightContrast01", 0.0), g.call("HeightContrast02", 0.0), g.call("HeightContrast03", 0.0), 0.0))
	m.set_shader_parameter("add", Vector4(0.0, g.call("Add_02", 0.0), g.call("Add_03", 0.0), g.call("Add_04", 0.0)))
	m.set_shader_parameter("contrast", Vector4(0.0, g.call("Contrast_02", 0.0), g.call("Contrast_03", 0.0), g.call("Contrast_04", 0.0)))
	m.set_shader_parameter("rough_mul", Vector4(g.call("01_RoughnessMultiply_01", 1.0), g.call("01_RoughnessMultiply_02", 1.0), g.call("01_RoughnessMultiply_03", 1.0), g.call("01_RoughnessMultiply_04", 1.0)))
	m.set_shader_parameter("mul_1", p.colors.get("01_Color_Multiply_01", Vector3.ONE))
	m.set_shader_parameter("overlay", p.colors.get("ColorOverlay", Vector3.ONE))
	m.set_shader_parameter("desaturation", g.call("Desaturation", 0.0))
	m.set_shader_parameter("bump", Vector4(g.call("BumpValue", 0.0), g.call("BumpValue_02", 0.0), g.call("BumpValue_03", 0.0), g.call("BumpValue_04", 0.0)))
	m.set_shader_parameter("bump_offset_power", g.call("BumpOffsetPower", 1.0))
	var n := 1
	for i in k.layers.size():
		var li: int = i + 2
		if not p.switches.get("UseLayer%d" % li, false):
			break
		m.set_shader_parameter("albedo_%d" % li, load(k.layers[i][0]))
		if k.layers[i][1] != "":
			m.set_shader_parameter("normal_%d" % li, load(k.layers[i][1]))
		if k.layers[i][2] != "":
			m.set_shader_parameter("packed_%d" % li, load(k.layers[i][2]))
		m.set_shader_parameter("mul_%d" % li, p.colors.get("01_Color_Multiply_0%d" % li, Vector3.ONE))
		n = li
	m.set_shader_parameter("layer_count", n)
	m.set_meta("ue_layers", n)

# Paint (static switch "UsePaintLayer?"), SHADERS.md 6b/6c: the mask's vertex channel is G on Arena_Blend_M and B on
# Architecture_M_New (fixed in the shader code); here only the instance's scalars and colors.
static func _paint(m: ShaderMaterial, p: Dictionary) -> void:
	if not p.switches.get("UsePaintLayer?", false):
		return
	m.set_shader_parameter("use_paint", true)
	m.set_shader_parameter("paint_power", p.scalars.get("PaintPower", 0.0))
	m.set_shader_parameter("paint_contrast1", p.scalars.get("PaintContrast_Color1", 0.0))
	m.set_shader_parameter("paint_contrast2", p.scalars.get("PaintContrast_Color2", 0.0))
	m.set_shader_parameter("curvature_cc", p.scalars.get("CurvatureColor", 1.0))
	m.set_shader_parameter("mask_output_level", p.scalars.get("MaskOutputLevel", 0.5))
	m.set_shader_parameter("paint_roughness", p.scalars.get("PaintRoughness", 0.0))
	m.set_shader_parameter("dhi", p.scalars.get("DetailLayer_HeightInfluence", 0.5))
	m.set_shader_parameter("color_inner", p.colors.get("Color_Inner", Vector3.ONE))
	m.set_shader_parameter("color_outer", p.colors.get("Color_Outer", Vector3(1, 0, 0)))
	m.set_shader_parameter("color_curvaturer", p.colors.get("Color_Curvaturer", Vector3.ONE))
	m.set_meta("ue_paint", true)

# Revision of ue_tint.gdshader: hash of its source (String.hash: https://docs.godotengine.org/en/stable/classes/class_string.html#class-string-method-hash).
static var _rev := -1
static func shader_rev() -> int:
	if _rev == -1:
		_rev = (load(SHADER_INC) as ShaderInclude).code.hash()
	return _rev

# Runtime: imported glbs carry the material (and its generated shader code) as it was at import time. Every surface
# material built by build() has meta ue_json (+ ue_shader_rev); this swaps each one whose revision is not the current
# shader's for a fresh build() of the same JSON, cached per JSON, so a shader / ue_material change needs no reimport.
# Covers MeshInstance3D surfaces (mesh and override materials) and MultiMesh meshes. Call before UeLevel's lightmap
# step (with_lightmap copies the rebuilt material). Returns materials replaced.
static func rebuild_all(root: Node) -> int:
	var cache := {}
	var fresh := func(m: Material) -> Material:
		if not (m is ShaderMaterial) or not m.has_meta("ue_json") or int(m.get_meta("ue_shader_rev", 0)) == shader_rev():
			return null
		var j: String = m.get_meta("ue_json")
		if not cache.has(j):
			cache[j] = build(j) if FileAccess.file_exists(j) else null
		return cache[j]
	var c := 0
	for gi in root.find_children("*", "GeometryInstance3D", true, false):
		var me: Mesh = null
		if gi is MeshInstance3D:
			me = gi.mesh
			if me:
				for s in me.get_surface_count():
					var o: Material = gi.get_surface_override_material(s)
					var r = fresh.call(o if o else me.surface_get_material(s))
					if r:
						if o: gi.set_surface_override_material(s, r)
						else: me.surface_set_material(s, r)
						c += 1
		elif gi is MultiMeshInstance3D and gi.multimesh and gi.multimesh.mesh:
			me = gi.multimesh.mesh
			for s in me.get_surface_count():
				var r = fresh.call(me.surface_get_material(s))
				if r:
					me.surface_set_material(s, r)
					c += 1
	return c

# Copy of a built material on the UE_LIGHTMAP variant with `lightmap_texture` set (the level builder's baked lighting;
# per-instance lm_* uniforms in ue_tint.gdshader). Resource.duplicate: https://docs.godotengine.org/en/stable/classes/class_resource.html
# sky_tex (optional): the SkyOcclusion atlas page for the same component.
static func with_lightmap(mat: ShaderMaterial, lightmap_tex: Texture2D, sky_tex: Texture2D = null) -> ShaderMaterial:
	var v: Array = mat.get_meta("ue_variant", [0, false, false, ""])
	var d := mat.duplicate() as ShaderMaterial
	set_variant(d, v[0], v[1], v[2], true, v[3] if v.size() > 3 else "")
	d.set_shader_parameter("lightmap_texture", lightmap_tex)
	if sky_tex:
		d.set_shader_parameter("sky_occlusion_texture", sky_tex)
		d.set_shader_parameter("use_sky_occlusion", true)
	return d

# Material JSON for a mesh's material slot. First the slot's real package (mesh package JSON: StaticMesh
# StaticMaterials[].MaterialInterface, SkeletalMesh SkeletalMaterials[].Material), then the folders near the mesh.
static func json_for(mesh_pkg: String, mat_name: String, glb_dir: String) -> String:
	# Sub-export meshes (HLOD proxies: …/HLOD/Arena_0_HLOD/SM_….glb in package …/HLOD/Arena_0_HLOD): read that package.
	var mp := UePkg.strip(mesh_pkg)
	var exports := []
	if FileAccess.file_exists(UePkg.root() + "/json/" + mp + ".json"):
		exports = UePkg.load_pkg(mp)
	else:
		var up := mp.get_base_dir()
		while up.contains("/") and not FileAccess.file_exists(UePkg.root() + "/json/" + up + ".json"):
			up = up.get_base_dir()
		if FileAccess.file_exists(UePkg.root() + "/json/" + up + ".json"):
			exports = [UePkg.export_named(UePkg.load_pkg(up), mp.get_file())]
	for e in exports:
		var slots: Array = e.get("Properties", {}).get("StaticMaterials", []) + e.get("SkeletalMaterials", [])
		for s in slots:
			var mi = s.get("MaterialInterface", s.get("Material"))
			if not (s is Dictionary) or not (mi is Dictionary):
				continue
			var on := String(mi.get("ObjectName", ""))
			var nm := on.get_slice("'", 1) if on.contains("'") else on
			if nm == mat_name:
				var j := DATA + _obj_pkg(mi.get("ObjectPath", "")) + ".json"
				if FileAccess.file_exists(j):
					return j
	return find_json(glb_dir, mat_name)

# Nearest M_*.json named `mat_name` for a mesh in `glb_dir`: the folder itself, then its subfolders, then the same for each
# parent up to `levels` up. Nearest wins, so Teutonic/SKeletal_Meshes finds Teutonic/M_Fallen_longsword.json and not the
# FallenSkin file of the same name. SkeletalMeshes/ finds the sibling MaterialInstances/INST_Longsword.json.
# DirAccess.get_directories_at: https://docs.godotengine.org/en/stable/classes/class_diraccess.html
static func find_json(glb_dir: String, mat_name: String, levels := 2) -> String:
	var dir := glb_dir.trim_suffix("/")
	for i in levels + 1:
		# Never search a whole category (Weapons/ holds every weapon, so a same-named file there is another weapon's).
		if i > 0 and dir.get_file() in ["Weapons", "Assets", "Content", "data"]:
			break
		var p := dir + "/" + mat_name + ".json"
		if FileAccess.file_exists(p):
			return p
		for sub in DirAccess.get_directories_at(dir):
			p = dir + "/" + sub + "/" + mat_name + ".json"
			if FileAccess.file_exists(p):
				return p
		dir = dir.get_base_dir()
	return ""

# ue_lightmap.gd - a level's baked lighting (UE MapBuildDataRegistry) attached to the placed meshes.
# Data path (all in the mdx json of <Level>_BuiltData, read by CUE4Parse UMapBuildDataRegistry,
# tools/CUE4Parse-src/CUE4Parse/UE4/Assets/Exports/BuildData/UMapBuildDataRegistry.cs):
#   Level.Properties.MapBuildData -> MapBuildDataRegistry export
#   component.LODData[0].MapBuildDataId -> MeshBuildData[id] (FMeshMapBuildData, :393-453)
#     .LightMap  = FLightMap2D (:498-603): Textures[0] (HQ LightMapTexture2D, top half = coefficient 0, bottom = 1),
#                  SkyOcclusionTexture, ScaleVectors[4]/AddVectors[4] (dequantization), CoordinateScale/Bias
#     .ShadowMap = FShadowMap2D (:605-...): Texture (ShadowMapTexture2D), CoordinateScale/Bias, bChannelValid[4],
#                  InvUniformPenumbraSize, LightGuids
#   LightBuildData[light guid].ShadowMapChannel (:190-201): which shadowmap channel a stationary light uses.
# Textures: `mdx exportlist` of the BuiltData package writes res://data/<package>/<export name>.png; their JSON says
# "SRGB": false (Arena_BuiltData.json, HQ_Lightmap0_* / ShadowMapTexture2D_*).
# The UV channel is StaticMesh.LightMapCoordinateIndex (mesh JSON).
#
# Shader side (godot/game/render/ue_tint.gdshader, owned by the materials builder) takes:
#   material: lightmap_texture (+ sky_occlusion_texture, see UeLevel.apply_lightmaps) via UeMaterial.with_lightmap
#   per instance (GeometryInstance3D.set_instance_shader_parameter): lm_coord = (CoordinateScale, CoordinateBias),
#   lm_scale0/lm_add0 = ScaleVectors[0]/AddVectors[0], lm_scale1/lm_add1 = ScaleVectors[1]/AddVectors[1].
class_name UeLightmap

const DATA := "res://data/"

static func v4(d) -> Vector4:
	return Vector4(d.get("X", 0.0), d.get("Y", 0.0), d.get("Z", 0.0), d.get("W", 0.0)) if d is Dictionary else Vector4()

static func v2(d) -> Vector2:
	return Vector2(d.get("X", 0.0), d.get("Y", 0.0)) if d is Dictionary else Vector2()

# The registry export of a level package, or {}.
static func registry(level_pkg: String) -> Dictionary:
	for e in UePkg.load_pkg(level_pkg):
		if e.get("Type", "") == "Level":
			var r := UeLevel.obj(e.get("Properties", {}).get("MapBuildData"))
			return r
	return {}

# Texture reference "pkg.N" -> res://data/<pkg>/<export N name>.png
static func tex_path(ref) -> String:
	if not (ref is Dictionary):
		return ""
	var e := UeLevel.obj(ref)
	return DATA + UeLevel.pkg_of(ref["ObjectPath"]) + "/" + String(e.get("Name", "")) + ".png" if not e.is_empty() else ""

# GUID string "C257E953-4C6040B1-..." -> registry key form "C257E9534C6040B1..."
static func key(guid: String) -> String:
	return guid.replace("-", "")

# Build data of one component, or {} (no LODData / not in the registry / no LightMap).
static func for_component(e: Dictionary, reg: Dictionary) -> Dictionary:
	var lod: Array = e.get("LODData", [])
	if lod.is_empty() or reg.is_empty() or not (lod[0] is Dictionary):
		return {}
	var id: String = lod[0].get("MapBuildDataId", "")
	var b: Dictionary = reg.get("MeshBuildData", {}).get(id, {})
	var lm = b.get("LightMap")
	if not (lm is Dictionary) or not lm.has("ScaleVectors"):
		return {}
	var out := {
		"id": id,
		"tex": tex_path(lm.get("Textures", [null])[0]),
		"sky": tex_path(lm.get("SkyOcclusionTexture")),
		"lm_coord": Vector4(v2(lm["CoordinateScale"]).x, v2(lm["CoordinateScale"]).y, v2(lm["CoordinateBias"]).x, v2(lm["CoordinateBias"]).y),
		"lm_scale0": v4(lm["ScaleVectors"][0]), "lm_add0": v4(lm["AddVectors"][0]),
		"lm_scale1": v4(lm["ScaleVectors"][1]), "lm_add1": v4(lm["AddVectors"][1]),
	}
	var sm = b.get("ShadowMap")
	if sm is Dictionary and sm.has("Texture"):
		var ch := -1
		var lbd: Dictionary = reg.get("LightBuildData", {})
		for g in sm.get("LightGuids", []):
			if lbd.has(key(g)) and int(lbd[key(g)].get("ShadowMapChannel", -1)) >= 0:
				ch = int(lbd[key(g)]["ShadowMapChannel"])
		out["sm_tex"] = tex_path(sm["Texture"])
		out["sm_coord"] = Vector4(v2(sm["CoordinateScale"]).x, v2(sm["CoordinateScale"]).y, v2(sm["CoordinateBias"]).x, v2(sm["CoordinateBias"]).y)
		out["sm_penumbra"] = v4(sm.get("InvUniformPenumbraSize"))
		out["sm_channel"] = ch
	return out

# StaticMesh.LightMapCoordinateIndex of a mesh (UV channel of the lightmap UVs); -1 if not serialized.
static func uv_index(mesh_obj_path: String) -> int:
	for e in UePkg.load_pkg(UeLevel.pkg_of(mesh_obj_path)):
		if e.get("Type", "") == "StaticMesh":
			return int(e.get("Properties", {}).get("LightMapCoordinateIndex", -1))
	return -1

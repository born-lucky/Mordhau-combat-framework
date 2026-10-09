# character_builder.gd - thin Godot adapter: a resolved Loadout -> one Node3D with the UMA master skeleton and every
# body part / wearable skeletal mesh bound to it (UE: UHumanMeshComponent builds the part list with
# SetupWearableConstruction_Internal, ported as Armor.construction(), and drives every part from one master pose).
# Godot equivalent of a master pose: each part's MeshInstance3D is moved under the master Skeleton3D and its Skin is
# rebound by bone name (Skin.set_bind_name; docs.godotengine.org/en/stable/classes/class_skin.html and
# class_meshinstance3d.html#property-skeleton). All parts share UMA_Master_Skeleton, so names match 1:1.
#
# Materials: a wearable's mesh slot is a master instance (e.g. M_EquipmentMasterChestShoulder_Masked_Inst) whose
# textures UE sets at runtime from the wearable class (UMordhauWearable AlbedoMap +0x68, NormalMap +0x70, RoughnessMap
# +0x78, Patterns[Pattern].Texture colour mask, colours from ColorTables). Those inputs are applied here with the shared
# ue_tint shader's M_WEARABLE mode (UeMaterial.wearable). Mask R -> colour 0 (ColorA), B -> colour 1 (ColorB),
# G -> colour 2 (ColorC) is CONFIRMED from the compiled master (docs/SHADERS.md 7); that the wearable's colour slots
# 0/1/2 feed ColorA/B/C in that order is UNCONFIRMED. The master has no Albedo/Metallic/Roughness scalars (SHADERS.md 7).
# Body parts keep the material the ue_import addon gives them on import.
class_name CharacterBuilder

const UMA_MESH := "Mordhau/Content/UMA/UMA/Master/UMA_Master"

# Result: Node3D "Character" > UMA_Master instance (its own body mesh hidden) whose Skeleton3D holds the parts.
# meta "missing": mesh paths that could not be loaded (pak backend: not in the paks; json: no exported glb, export
# them with scripts/mdx.sh export). Meshes come through UeAnim (paks or glb, per the data backend).
static func build(l: Loadout) -> Node3D:
	var root := Node3D.new()
	root.name = "Character"
	var master := UeAnim.instance(UMA_MESH)
	if master == null:
		push_error("CharacterBuilder: UMA_Master glb not imported")
		return root
	master.name = "Body"
	root.add_child(master)
	var skel := UeAnim.skeleton_of(master)
	for mi in _meshes(skel):
		mi.visible = false			# master pose only; the visible body comes from the separated parts
	var ap := l.appearance
	var face := UeWearable.face_def(ap.b_is_female, ap.face)
	var hair := ""
	var facial_hair := ""
	if face != null:
		if ap.hair >= 0 and ap.hair < face.hair.size():
			hair = UeWearable.hair_mesh(face.hair[ap.hair])
		if ap.facial_hair >= 0 and ap.facial_hair < face.facial_hair.size():
			facial_hair = UeWearable.hair_mesh(face.facial_hair[ap.facial_hair])
	var owner := {}		# mesh path -> slot of the wearable that brought it
	for slot in Loadout.SLOTS:
		var w := l.wearable(slot)
		if w != null:
			for m in [w.mesh, w.aux_mesh]:
				if m != "":
					owner[m] = slot
	var missing := PackedStringArray()
	for p in Armor.construction(l, face, hair, facial_hair):
		var part: Node = UeAnim.mesh_node(p, UeWearable.glb(p))	# from the paks or the glb, per the data backend
		if part == null:
			missing.append(p)
			continue
		for mi in _attach(part, skel):
			mi.name = p.get_file()
			if owner.has(p):
				_paint(mi, l, owner[p])
		part.free()
	root.set_meta("missing", missing)
	return root

static func _meshes(n: Node) -> Array:
	var out := []
	if n is MeshInstance3D:
		out.append(n)
	for c in n.get_children():
		out += _meshes(c)
	return out

# Move every skinned MeshInstance3D of `part` under `skel`, rebinding its skin by bone name.
static func _attach(part: Node, skel: Skeleton3D) -> Array:
	var own := UeAnim.skeleton_of(part)
	var out := []
	for mi: MeshInstance3D in _meshes(part):
		var xf := _rel(mi, own)
		var skin := mi.skin
		if skin != null and own != null:
			skin = skin.duplicate()
			for b in skin.get_bind_count():
				if String(skin.get_bind_name(b)) == "":
					skin.set_bind_name(b, own.get_bone_name(skin.get_bind_bone(b)))
		mi.get_parent().remove_child(mi)
		mi.owner = null
		skel.add_child(mi)
		mi.transform = xf
		mi.skin = skin
		mi.skeleton = NodePath("..")
		out.append(mi)
	return out

# transform of n relative to `to` (walks parents; both in the same unattached scene)
static func _rel(n: Node3D, to: Node3D) -> Transform3D:
	var x := Transform3D.IDENTITY
	var p: Node = n
	while p != null and p != to:
		if p is Node3D:
			x = (p as Node3D).transform * x
		p = p.get_parent()
	return x

# Colour of colour slot i of the wearable in `slot`: Singleton.ColorTables[w.ColorTables[i]].Entries[Colors[i]]
# (FWearableCustomization.Colors copied onto the wearable instance, AMordhauCharacter::UpdateWearableInstanceColorsAndPatterns
# rva=0x1575d90). UseColorsFromSlot (+0x60, 10 = Invalid) borrows another slot's colour indexes.
static func colors_of(l: Loadout, slot: int) -> Array:
	var w := l.wearable(slot)
	var out := []
	if w == null:
		return out
	var src := slot if w.use_colors_from_slot >= Loadout.SLOTS else w.use_colors_from_slot
	var idx: PackedInt32Array = l.colors[src] if src < l.colors.size() else PackedInt32Array()
	for i in w.color_tables.size():
		out.append(UeWearable.color(w.color_tables[i], idx[i] if i < idx.size() else 0))
	return out

static func _paint(mi: MeshInstance3D, l: Loadout, slot: int) -> void:
	var w := l.wearable(slot)
	var alb := UeWearable.png(w.albedo_map)
	if alb == "":
		return			# textures not exported: keep the imported material
	# SHADERS.md 7 (M_EquipmentMasterChestShoulder): UeMaterial.wearable = Bleed 1.5 weights, mask R -> ColorA,
	# B -> ColorB, G -> ColorC, each multiplying the untinted albedo. Only the primary set: WearableData keeps no secondary
	# textures or emblem. Which wearable fills the Secondary* set is set natively (likely
	# UHumanMeshComponent::SetMaterialParamsForMergedSlot rva=0x14d3490, UNCONFIRMED, not ported), so has_secondary
	# stays off and every vertex uses this wearable's set.
	var tex := func(path: String) -> Texture2D: return load(path) if path != "" else null
	var pi := l.patterns[slot]
	var cols := [Vector3(1, 0, 0), Vector3.ZERO, Vector3.ONE]		# master defaults ColorA/B/C (SHADERS.md 7)
	var cs := colors_of(l, slot)
	for i in mini(cs.size(), 3):
		var c: Color = cs[i]
		cols[i] = Vector3(c.r, c.g, c.b)
	var primary := {"albedo": load(alb), "normal": tex.call(UeWearable.png(w.normal_map)),
		"rma": tex.call(UeWearable.png(w.roughness_map)),
		"mask": tex.call(UeWearable.png(w.patterns[pi]) if pi < w.patterns.size() else ""), "colors": cols}
	for s in mi.mesh.get_surface_count():
		var cur := mi.mesh.surface_get_material(s)
		var masked := cur != null and cur.resource_name.contains("_Masked")
		var m := UeMaterial.wearable(primary, {}, {}, masked)
		m.resource_name = w.id
		if masked:
			m.set_shader_parameter("alpha_from_albedo", UeMaterial.alpha_is_mask(alb, 0.3333))
		mi.set_surface_override_material(s, m)

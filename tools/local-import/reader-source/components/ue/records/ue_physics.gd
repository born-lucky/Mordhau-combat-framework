# ue_physics.gd - collision data the melee trace reads, from the user's extract/json:
#   * PhysicsAsset bodies (SkeletalBodySetup.AggGeom.BoxElems per bone): what a weapon trace hits on a character.
#     The character mesh's PhysicsAssetOverride is UMA_Master_PhysicsAsset (BP_MordhauCharacter CDO,
#     CharacterMesh0.PhysicsAssetOverride; 16 bodies, one or two boxes each).
#   * Skeletal mesh / skeleton sockets (SkeletalMeshSocket: SocketName, BoneName, RelativeLocation): the weapon's
#     TraceStart / TraceEnd points (names from AMordhauWeapon::GetTrace_Implementation rva=0x1629520, which reads the
#     FName strings at .rdata 0x144361d18 "TraceStart", 0x144361d40 "TraceEnd", 0x144361d28 "SecondTraceStart",
#     0x144361d50 "SecondTraceEnd"; Second* when bIsUsingAlternateMode, weapon +0xd22).
# Everything is returned in Godot bone space (metres, Y-up), converted with the same rule the glTF writer applies to
# bone rest transforms (swap / quat, x0.01; docs/WORKFLOW_SOURCES.md section 0), so a box or socket placed
# on a bone's Godot global transform lands where UE puts it.
class_name UePhysics

const CHARACTER_PA := "Mordhau/Content/UMA/UMA/Master/UMA_Master_PhysicsAsset"

static var _cache := {}

# UE -> Godot conversion, the same rule the glTF writer applies (CUE4Parse Gltf.cs SwapYZ, x0.01): kept local so this
# reader does not depend on the map builder's ue_level.gd.
static func swap(v: Dictionary) -> Vector3:
	return Vector3(v.get("X", 0.0), v.get("Z", 0.0), v.get("Y", 0.0))

# CUE4Parse FRotator.Quaternion (FRotator.cs:88-111): degrees in, UE quaternion (x, y, z, w) out
static func rot_quat(r: Dictionary) -> Array:
	var h := PI / 360.0
	var p: float = r.get("Pitch", 0.0) * h
	var y: float = r.get("Yaw", 0.0) * h
	var o: float = r.get("Roll", 0.0) * h
	var sp := sin(p); var cp := cos(p)
	var sy := sin(y); var cy := cos(y)
	var sr := sin(o); var cr := cos(o)
	return [cr * sp * sy - sr * cp * cy, -cr * sp * cy - sr * cp * sy, cr * cp * sy - sr * sp * cy, cr * cp * cy + sr * sp * sy]

# UE quaternion -> Godot via SwapYZ(FQuat) = (x, z, y, -w) (Gltf.cs:250)
static func quat(q: Array) -> Quaternion:
	return Quaternion(q[0], q[2], q[1], -q[3]).normalized()

# A UE location (cm) and FRotator (pitch, yaw, roll; degrees) as typed values -> Godot transform, by the same rules
# (swap x 0.01, rot_quat, quat); unit scale
static func ue_xf(loc: Vector3, rot := Vector3.ZERO) -> Transform3D:
	var q := quat(rot_quat({"Pitch": rot.x, "Yaw": rot.y, "Roll": rot.z}))
	return Transform3D(Basis(q), Vector3(loc.x, loc.z, loc.y) * 0.01)

# [{bone, xf: Transform3D (box centre + rotation, bone space), half: Vector3 (half extents, m)}]
# FKBoxElem X/Y/Z are the box's full side lengths. UNCONFIRMED: engine semantics (UE 4.26 FKBoxElem), not in repo.
static func body_boxes(pa_path := CHARACTER_PA) -> Array:
	var key := "pa:" + pa_path
	if _cache.has(key):
		return _cache[key]
	var out := []
	for e in UePkg.load_pkg(pa_path):
		if e.get("Type", "") != "SkeletalBodySetup":
			continue
		var p: Dictionary = e.get("Properties", {})
		for b in p.get("AggGeom", {}).get("BoxElems", []):
			if String(b.get("CollisionEnabled", "ECollisionEnabled::QueryAndPhysics")).contains("NoCollision"):
				continue
			var xf := Transform3D(Basis(quat(rot_quat(b.get("Rotation", {})))),
				swap(b.get("Center", {})) * 0.01)
			var half := Vector3(b.get("X", 0.0), b.get("Z", 0.0), b.get("Y", 0.0)) * 0.005
			out.append({"bone": String(p.get("BoneName", "")), "xf": xf, "half": half})
	_cache[key] = out
	return out

# {socket name: {bone, loc: Vector3 (bone space, m)}} from a SkeletalMesh package: the skeleton's sockets, then the
# mesh's own on top. UNCONFIRMED: mesh sockets override skeleton sockets of the same name (USkeletalMesh::FindSocket,
# engine code not in repo).
static func sockets(mesh_path: String) -> Dictionary:
	var key := "so:" + mesh_path
	if _cache.has(key):
		return _cache[key]
	var out := {}
	var pkg := UePkg.load_pkg(UePkg.strip(mesh_path))
	var skel := ""
	for e in pkg:
		if e.get("Type", "") == "SkeletalMesh":
			skel = UePkg.strip(String(e.get("Properties", {}).get("Skeleton", {}).get("ObjectPath", "")))
	if skel != "":
		_add_sockets(UePkg.load_pkg(skel), out)
	_add_sockets(pkg, out)
	_cache[key] = out
	return out

static func _add_sockets(pkg: Array, out: Dictionary) -> void:
	for e in pkg:
		if e.get("Type", "") != "SkeletalMeshSocket":
			continue
		var p: Dictionary = e.get("Properties", {})
		out[String(p.get("SocketName", ""))] = {"bone": String(p.get("BoneName", "")),
			"loc": swap(p.get("RelativeLocation", {})) * 0.01}

# One SkeletalMeshSocket, bone space (m)
class Socket:
	var bone := ""
	var loc := Vector3.ZERO

# FName strings AMordhauWeapon::GetTrace_Implementation reads (see header)
const TRACE_START := "TraceStart"
const TRACE_END := "TraceEnd"

# a socket of a skeletal mesh (sockets() rules), null when the mesh has none of that name
static func socket(mesh_path: String, socket_name: String) -> Socket:
	if mesh_path == "":
		return null
	var s = sockets(mesh_path).get(socket_name)
	if s == null:
		return null
	var out := Socket.new()
	out.bone = s.bone
	out.loc = s.loc
	return out

# The skeletal mesh whose skeleton carries a weapon's trace sockets: the weapon's own mesh, else the first part mesh
# of its first skin (Longsword: SK_Longsword_Blade_03 -> skeleton Armature_Longswords).
static func weapon_mesh(w: WeaponData) -> String:
	if w.mesh != "":
		return w.mesh
	for s in UeWeapon.skins(w):
		for pt in s.part_types:
			for part in pt.parts:
				if part.mesh != "":
					return part.mesh
	return ""

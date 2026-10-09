# anim_math.gd - value math shared by the animation port (no Node, no scene: plain functions on Quaternion/floats).
#
# Axes: UE component space is X fwd, Y right, Z up (left-handed, cm); Godot skeleton space is the glTF conversion of
# it, CUE4Parse Gltf.SwapYZ: vector (x, y, z) -> (x, z, y) * 0.01, quaternion (x, y, z, w) -> (x, z, y, -w)
# (tools/CUE4Parse-src/CUE4Parse-Conversion/Writers/Gltf/Gltf.cs 244-250, also used by `mdx anim` for every key).
# The swap is a reflection, so a rotation R composed in UE component space maps to SwapYZ(R) composed the same way
# in Godot skeleton space; the ported node math is therefore done directly in Godot space on converted rotations.
class_name AnimMath

const SMALL := 1e-8		# UE SMALL_NUMBER

# FRotator::Quaternion (engine, VA 0x1418b9f90, the func_0x1418b9f90 of the Ghidra C; disassembled): angles wound
# with VectorMod 360, times DEG_TO_RAD / 2, VectorSinCos (sin polynomial with 1/9! = 2.7557e-6 at 0x144070230, cos
# with 1/8! at 0x144070240); then LeftTerm = CR * [SP*SY, SP*CY, CP*SY, CP*CY] ^ (+,-,+,+) (mask 0x14449f8f0),
# RightTerm = SR * [CP*CY, CP*SY, SP*CY, SP*SY] ^ (-,-,-,+) (mask 0x14449f910), Result = Left + Right:
#   X =  CR*SP*SY - SR*CP*CY ; Y = -CR*SP*CY - SR*CP*SY ; Z = CR*CP*SY - SR*SP*CY ; W = CR*CP*CY + SR*SP*SY
#   Its two vector constants are .bss globals filled at static init (FRotator::Quaternion loads them at 0x1418ba010 and
#   0x1418b9f94): DEG_TO_RAD_HALF (.bss 0x14577a680) and Float360 (.bss 0x14577a670), written by these initializers:
const _DEG_TO_RAD_HALF := PI / 180.0 * 0.5	# GlobalVectorConstants::`dynamic initializer for 'DEG_TO_RAD_HALF'' 0x140669cc0 copies 0x143fe0d70 (0.008726646)
const _F360 := 360.0	# GlobalVectorConstants::`dynamic initializer for 'Float360'' 0x140669d00 copies 0x143fe0eb0 (360)

static func rotator_quat_ue(pitch: float, yaw: float, roll: float) -> Quaternion:
	var h := _DEG_TO_RAD_HALF
	var p := fmod(pitch, _F360) * h
	var y := fmod(yaw, _F360) * h
	var r := fmod(roll, _F360) * h
	var sp := sin(p); var cp := cos(p)
	var sy := sin(y); var cy := cos(y)
	var sr := sin(r); var cr := cos(r)
	return Quaternion(cr * sp * sy - sr * cp * cy, -cr * sp * cy - sr * cp * sy, cr * cp * sy - sr * sp * cy,
		cr * cp * cy + sr * sp * sy)

# Gltf.SwapYZ(FQuat) = (X, Z, Y, -W)
static func quat_ue_to_godot(q: Quaternion) -> Quaternion:
	return Quaternion(q.x, q.z, q.y, -q.w)

static func quat_godot_to_ue(q: Quaternion) -> Quaternion:
	return Quaternion(q.x, q.z, q.y, -q.w)

static func vec_godot_to_ue(v: Vector3) -> Vector3:
	return Vector3(v.x, v.z, v.y) * 100.0

# FRotator (Vector3 = Pitch, Yaw, Roll in degrees) -> Godot-space rotation
static func rotator_quat(r: Vector3) -> Quaternion:
	return quat_ue_to_godot(rotator_quat_ue(r.x, r.y, r.z))

# FQuat::Rotator yaw (engine, VA 0x1418bdc80, disassembled): YawY = 2(WZ + XY), YawX = 1 - 2(Y^2 + Z^2),
# Yaw = atan2(YawY, YawX) * 57.2958 on every branch (the pitch singularity branches only change pitch / roll).
static func quat_yaw_ue(q: Quaternion) -> float:
	return rad_to_deg(atan2(2.0 * (q.w * q.z + q.x * q.y), 1.0 - 2.0 * (q.y * q.y + q.z * q.z)))

# FRotator::NormalizeAxis: wrap to (-180, 180]
static func normalize_axis(a: float) -> float:
	var r := fmod(a, 360.0)
	if r < 0.0:
		r += 360.0
	if r > 180.0:
		r -= 360.0
	return r

# FMath::SmoothStep(0, 1, x) (engine, VA 0x1414d7960, disassembled): x < A -> 0, x >= B -> 1, else t = (x - A) /
# (B - A), (3 - 2t) t^2
static func smoothstep01(x: float) -> float:
	var t := clampf(x, 0.0, 1.0)
	return t * t * (3.0 - 2.0 * t)

# UMordhauUtilityLibrary::GetNormalizedTime rva=0x1624620: Current < End ? (Current <= Start ? 0 :
# (Start < End ? (Current - Start) / (End - Start) : 1)) : 1
static func normalized_time(start: float, end: float, current: float) -> float:
	if current < end:
		if current <= start:
			return 0.0
		if start < end:
			return (current - start) / (end - start)
	return 1.0

# EAlphaBlendOption names (PDB LF_ENUM order; the PlayAttackAnim call passes 2 = HermiteCubic, 14 = Custom)
const BLEND_OPTIONS := ["Linear", "Cubic", "HermiteCubic", "Sinusoidal", "QuadraticInOut", "CubicInOut",
	"QuarticInOut", "QuinticInOut", "CircularIn", "CircularOut", "CircularInOut", "ExpIn", "ExpOut", "ExpInOut", "Custom"]

static func blend_option(v) -> int:
	if v is int:
		return v
	var s := String(v)
	s = s.get_slice("::", s.count("::"))
	var i := BLEND_OPTIONS.find(s)
	if i < 0:
		push_error("AnimMath.blend_option: unknown EAlphaBlendOption %s" % s)
	return i

# FAlphaBlend::AlphaToBlendOption (engine, .text 0x2e46870 = VA 0x142e47870; disassembled with scripts/ue_dis.py,
# jump table at 0x142e47d2c indexed by option - 1, option 0 / out of range = the Linear tail at 0x142e47d01).
# Every case clamps its result to [0, 1] (comiss 0 / minss 1):
#   Linear a | Cubic 3a^2 - 2a^3 (0x142e478f6) | HermiteCubic: a < 0 -> 0, a >= 1 -> 1, else (3 - 2a) a^2 (0x9c8)
#   Sinusoidal (sin(a*PI - PI/2) + 1) * 0.5 (0x8af) | *InOut n=2..5: x = 2a; a < 0.5 ? x^n / 2 : (2 - (2 - x)^n) / 2
#   CircularIn 1 - sqrt(1 - a^2) | CircularOut sqrt(1 - (a - 1)^2) | CircularInOut x = 2a; a < 0.5 ?
#   (1 - sqrt(1 - x^2)) / 2 : (sqrt(1 - (x - 2)^2) + 1) / 2 | ExpIn a == 0 ? 0 : 2^(10(a - 1)) | ExpOut a == 1 ? 1 :
#   1 - 2^(-10a) | ExpInOut x = 2a; a < 0.5 ? (x == 0 ? 0 : 2^(10(x - 1))) / 2 : (x - 1 == 1 ? 1 : 2 - 2^(-10(x - 1))) / 2
#   Custom (0x142e47cbc): no curve -> Linear; else curve(TimeMin + (TimeMax - TimeMin) * a)
#   (UCurveBase::GetTimeRange = first / last key time, then UCurveFloat::GetFloatValue).
static func alpha_blend(option: int, alpha: float, curve := "") -> float:
	var a := alpha
	var v := a
	match option:
		1: v = 3.0 * a * a - 2.0 * a * a * a
		2:
			if a < 0.0: v = 0.0
			elif a >= 1.0: v = 1.0
			else: v = (3.0 - 2.0 * a) * a * a
		3: v = (sin(a * PI - PI * 0.5) + 1.0) * 0.5
		4, 5, 6, 7:
			var n := float(option - 2)
			var x := a + a
			v = pow(x, n) * 0.5 if a < 0.5 else (2.0 - pow(2.0 - x, n)) * 0.5
		8: v = 1.0 - sqrt(maxf(1.0 - a * a, 0.0))
		9: v = sqrt(maxf(1.0 - (a - 1.0) * (a - 1.0), 0.0))
		10:
			var x2 := a + a
			v = (1.0 - sqrt(maxf(1.0 - x2 * x2, 0.0))) * 0.5 if a < 0.5 else (sqrt(maxf(1.0 - (x2 - 2.0) * (x2 - 2.0), 0.0)) + 1.0) * 0.5
		11: v = 0.0 if a == 0.0 else pow(2.0, (a - 1.0) * 10.0)
		12: v = 1.0 if a == 1.0 else 1.0 - pow(2.0, a * -10.0)
		13:
			var x3 := a + a
			if a < 0.5:
				v = (0.0 if x3 == 0.0 else pow(2.0, (x3 - 1.0) * 10.0)) * 0.5
			else:
				var x4 := x3 - 1.0
				v = ((1.0 if x4 == 1.0 else 1.0 - pow(2.0, x4 * -10.0)) + 1.0) * 0.5
		14:
			if curve != "":
				var ks := CombatData.curve_keys(curve)
				var t0: float = float(ks[0].Time) if ks.size() > 0 else 0.0
				var t1: float = float(ks[ks.size() - 1].Time) if ks.size() > 0 else 0.0
				v = CombatData.curve_value(curve, t0 + (t1 - t0) * a)
	return clampf(v, 0.0, 1.0)

# FAnimationRuntime::BlendPosesTogether / BlendPosesTogetherIndirect (engine, VA 0x142e4a250, 0x142e4a3c0 / 0x142e4a6b0;
# disassembled): pose 0 -> BlendTransformOverwrite, poses 1.. -> BlendTransformAccumulate, then
# FCompactHeapPose::NormalizeRotations (0x142eda520) only when there is more than one pose. Both kernels sit behind
# CPU-level dispatchers with no PDB symbol (the names Overwrite / Accumulate are UE 4.26 source names, UNCONFIRMED
# in the exe; the roles follow from the call order: the per-pose loop 0x142e4a2c0..0x142e4a2df in
# FAnimationRuntime::BlendPosesTogether calls the second one). Level-0 kernels read:
#   unnamed code at 0x1402950e0 called from `FAnimationRuntime::BlendPosesTogether` at 0x142e4a2d0 via 0x140297080 (accumulate)
#   unnamed code at 0x1404d5f20 called from `FCompactPose::NormalizeRotations` at 0x142eda526 via 0x1404d63c0 (normalize)
#   Overwrite: Out = w * (Rotation, Translation, Scale3D)
#   Accumulate: R = w * Src.Rotation; Out.Rotation += (dot(R, Out.Rotation) >= 0 ? R : -R) (sign against the running
#     sum, not the first pose); Out.Translation += w * T; Out.Scale3D += w * S
#   NormalizeRotations: |q|^2 >= 1e-8 (0x143fe0d40) ? q * rsqrt(|q|^2) (one Newton step) : identity (0x143fe0de0).
# Zero weights are kept (they add nothing). `xs` / `ws` same length.
static func blend_transforms(xs: Array, ws: PackedFloat32Array) -> Transform3D:
	if xs.is_empty():
		return Transform3D.IDENTITY
	var t := Vector3.ZERO
	var s := Vector3.ZERO
	var q := Quaternion(0, 0, 0, 0)
	for i in xs.size():
		var w := ws[i]
		var x: Transform3D = xs[i]
		var r := x.basis.get_rotation_quaternion()
		var rw := Quaternion(r.x * w, r.y * w, r.z * w, r.w * w)
		if i > 0 and rw.dot(q) < 0.0:
			rw = -rw
		q = Quaternion(q.x + rw.x, q.y + rw.y, q.z + rw.z, q.w + rw.w)
		t += x.origin * w
		s += x.basis.get_scale() * w
	if xs.size() > 1:
		q = q.normalized() if q.length_squared() >= SMALL else Quaternion.IDENTITY
	elif q.length_squared() < SMALL:
		q = Quaternion.IDENTITY		# Godot cannot hold a zero rotation (a single pose at weight ~0)
	else:
		q = q.normalized()			# Godot's Basis needs a unit quaternion; the |w| < 1 shrink is lost (UNCONFIRMED effect)
	return compose(q, s, t)

static func compose(rot: Quaternion, scale: Vector3, origin: Vector3) -> Transform3D:
	return Transform3D(Basis(rot) * Basis.from_scale(scale), origin)

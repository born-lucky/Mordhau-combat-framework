# ue_sound.gd - UE4 SoundCue / SoundAttenuation (mdx json) -> playable Godot audio.
#
# Sources. Native defaults are not in any package JSON (cooked tags hold only non-default values, see ue_pkg.gd), so
# they were read from the shipping exe's constructors (RVA = .text offset in extract/native/publics.tsv + 0x1000),
# with field names mapped by the exe's UHT property tables (name -> struct offset):
#   USoundNodeModulator ctor   0x3450e10: PitchMin@0x48=0.95 PitchMax@0x4c=1.05 VolumeMin@0x50=0.95 VolumeMax@0x54=1.05
#   USoundNodeModulator::ParseNodes 0x3467ca0: once per active sound v=Max+(Min-Max)*u, u=FRandomStream [0,1);
#                                     ParseParams.Volume*=v(Volume), .Pitch*=v(Pitch)
#   USoundNodeRandom ctor      0x3450f30: bRandomizeWithoutReplacement=true (bitfield 0x70|4), Weights empty
#   USoundNodeRandom::ChooseNodeIndex 0x3458280: r=FRand*sum(weights of unused), first child whose running sum > r
#   USoundNodeMixer::ParseNodes 0x3467b80: every child, Volume*=InputVolume[i]
#   USoundNodeSwitch::ParseNodes 0x3468690: i = param found ? value+1 : 0; out of range -> 0
#   USoundNodeBranch::ParseNodes 0x3465ba0: param unset -> child 2, true -> 0, false -> 1
#   USoundNodeDelay::ParseNodes 0x34660a0: delay = max(DelayMax+(DelayMin-DelayMax)*u, 0)
#   USoundNodeModulatorContinuous ctor 0x3450e50 + FModulatorContinuousParams::GetValue 0x3461ef0:
#       Default=1 MinInput=0 MaxInput=1 MinOutput=0 MaxOutput=1 ParamMode=0; v=param or Default; mode 2 -> v;
#       mode 1 -> |v|; then clamp(v,MinIn,MaxIn) mapped linearly to MinOut..MaxOut (slope 0 if MaxIn<=MinIn);
#       UEnum table: MPM_Normal/MPM_Abs/MPM_Direct = 0/1/2
#   USoundCue ctor             0x3450830: VolumeMultiplier@0x1c8=0.75 PitchMultiplier@0x1cc=1 (= CUE4Parse USoundCue.cs:20-21)
#   USoundWave ctor            Volume@0x254=1 Pitch@0x258=1
#   FBaseAttenuationSettings ctor 0x2ee93a0: DistanceAlgorithm@8=Linear Shape@9=Sphere dBAttenuationAtMax@0xc=-60
#       FalloffMode@0x10=Continues AttenuationShapeExtents@0x14=(400,0,0) ConeOffset@0x20=0 FalloffDistance@0x24=3600
#   FSoundAttenuationSettings ctor 0x2ca8c60: LPFRadiusMin=3000 LPFRadiusMax=6000 LPFFrequencyAtMin/AtMax=20000,
#       StereoSpread=200 OmniRadius=0; bitfield bits0,1 set (bAttenuate, bSpatialize), bits2..6 clear
#   FBaseAttenuationSettings::Evaluate 0x2ef26a0 (Sphere): d = max(|listener-origin| - Extents.X, 0)
#   FBaseAttenuationSettings::AttenuationEval 0x2eed6f0: f=max(Falloff,1), a=d/f, result clamped to [0,1]:
#       Linear 1-a | Logarithmic -0.5*ln(max(d,1e-4)/f) | Inverse 0.02/(max(d,1e-4)/f)
#       LogReverse d>f ? 0 : 1+0.5*ln(max(1-a,1e-4)) | NaturalSound 10^(a*dBAttenuationAtMax/20)
#       (FalloffMode Silent: a>=1 -> 0; Hold: a clamped to 1) | Custom: FRichCurve (not ported)
#   Enum values come from the exe's UEnum tables: EAttenuationDistanceModel Linear..Custom = 0..5,
#   ENaturalSoundFalloffMode Continues/Silent/Hold = 0/1/2, EAttenuationShape Sphere/Capsule/Box/Cone = 0..3.
# Godot side (docs.godotengine.org): AudioStreamRandomizer (random_pitch: pitch in [1/r, r]; random_volume_offset_db:
#   +- offset; add_stream(index, stream, weight)), AudioStreamPlayer3D (ATTENUATION_DISABLED = no distance attenuation;
#   max_distance 0 = no cutoff; attenuation_filter_cutoff_hz 20500 disables the filter), Viewport.get_audio_listener_3d,
#   Viewport.get_camera_3d, AudioStreamOggVorbis.load_from_file, AudioStreamWAV.load_from_file.
#
# Use: var d := UeSound.cue("Mordhau/Content/.../SC_Hit_ChopMedium.0")
#      var s := UeSound.new(); s.desc = d; add_child(s); s.play_cue({"Reason": 2})
# The node computes UE's own falloff curve every frame (Godot has no NaturalSound model), see apply_player().
class_name UeSound
extends Node3D

const DATA := "res://data/"
const CM := 0.01					# UE cm -> Godot m (docs/DESIGN.md §4)

const MOD_DEF := {"PitchMin": 0.95, "PitchMax": 1.05, "VolumeMin": 0.95, "VolumeMax": 1.05}
const MC_DEF := {"Default": 1.0, "MinInput": 0.0, "MaxInput": 1.0, "MinOutput": 0.0, "MaxOutput": 1.0}
const ATT_DEF := {
	"DistanceAlgorithm": "EAttenuationDistanceModel::Linear", "AttenuationShape": "EAttenuationShape::Sphere",
	"dBAttenuationAtMax": -60.0, "FalloffMode": "ENaturalSoundFalloffMode::Continues",
	"AttenuationShapeExtents": {"X": 400.0, "Y": 0.0, "Z": 0.0}, "ConeOffset": 0.0, "FalloffDistance": 3600.0,
	"bAttenuate": true, "bSpatialize": true, "bAttenuateWithLPF": false,
	"LPFRadiusMin": 3000.0, "LPFRadiusMax": 6000.0, "LPFFrequencyAtMin": 20000.0, "LPFFrequencyAtMax": 20000.0,
}
# node types whose ParseNodes was read in the exe; anything else follows its first child and marks the cue inexact
const KNOWN := ["WavePlayer", "Modulator", "Random", "Mixer", "Switch", "Branch", "Delay", "ModulatorContinuous"]

static var _cues := {}
static var _streams := {}

# ---------------------------------------------------------------- data

# SoundCue package -> {path, volume, pitch, attenuation, root, waves, exact, unknown}
static func cue(obj_path: String) -> Dictionary:
	var p := UePkg.strip(obj_path)
	if _cues.has(p):
		return _cues[p]
	var pkg := UePkg.load_pkg(p)
	var c := UePkg.export_of(pkg, "SoundCue")
	if c.is_empty():
		return {}
	var pr: Dictionary = c.get("Properties", {})
	var d := {"path": p, "volume": float(pr.get("VolumeMultiplier", 0.75)), "pitch": float(pr.get("PitchMultiplier", 1.0)),
		"waves": PackedStringArray(), "unknown": PackedStringArray()}
	if pr.get("bOverrideAttenuation", false):
		d.attenuation = attenuation_settings(pr.get("AttenuationOverrides", {}))
	elif pr.has("AttenuationSettings"):
		d.attenuation = attenuation(String(pr.AttenuationSettings.ObjectPath))
	else:
		d.attenuation = {}
	d.root = _node(pkg, pr.get("FirstNode"), d)
	d.exact = d.unknown.is_empty()
	_cues[p] = d
	return d

static func _node(pkg: Array, ref, d: Dictionary) -> Dictionary:
	if not (ref is Dictionary) or not ref.has("ObjectName"):
		return {}
	var e := UePkg.export_named(pkg, String(ref.ObjectName).get_slice("'", 1).get_slice(":", 1))
	var t := String(e.get("Type", "")).trim_prefix("SoundNode")
	var pr: Dictionary = e.get("Properties", {})
	var n := {"type": t, "children": []}
	for ch in pr.get("ChildNodes", []):
		n.children.append(_node(pkg, ch, d))
	match t:
		"WavePlayer":
			var w := UePkg.strip(String(e.get("SoundWave", {}).get("ObjectPath", "")))
			if w == "":		# soft path "/Game/X.X" -> package "Mordhau/Content/X"
				w = "Mordhau/Content/" + String(pr.get("SoundWaveAssetPtr", {}).get("AssetPathName", "")).trim_prefix("/Game/").get_basename()
			n.wave = w
			n.looping = bool(pr.get("bLooping", false))
			if not d.waves.has(w):
				d.waves.append(w)
		"Modulator":
			for k in MOD_DEF:
				n[k] = float(pr.get(k, MOD_DEF[k]))
		"Random":
			n.weights = PackedFloat32Array(pr.get("Weights", []))
			n.no_repeat = bool(pr.get("bRandomizeWithoutReplacement", true))
			n.used = []
			n.used.resize(n.children.size()); n.used.fill(false)
		"Mixer":
			n.input_volume = PackedFloat32Array(pr.get("InputVolume", []))
		"Switch":
			n.param = String(pr.get("IntParameterName", ""))
		"Branch":
			n.param = String(pr.get("BoolParameterName", ""))
		"Delay":
			n.DelayMin = float(pr.get("DelayMin", 0.0)); n.DelayMax = float(pr.get("DelayMax", 0.0))
		"ModulatorContinuous":
			n.pitch_mod = _mc(pr.get("PitchModulationParams", {}))
			n.volume_mod = _mc(pr.get("VolumeModulationParams", {}))
		_:
			if not d.unknown.has(t):
				d.unknown.append(t)
	return n

static func _mc(s: Dictionary) -> Dictionary:
	var m := {"param": String(s.get("ParameterName", "")), "mode": String(s.get("ParamMode", ""))}
	for k in MC_DEF:
		m[k] = float(s.get(k, MC_DEF[k]))
	return m

# SoundAttenuation package -> settings with native defaults filled + metre values
static func attenuation(obj_path: String) -> Dictionary:
	var e := UePkg.export_of(UePkg.load_pkg(obj_path), "SoundAttenuation")
	return attenuation_settings(e.get("Properties", {}).get("Attenuation", {}))

static func attenuation_settings(s: Dictionary) -> Dictionary:
	var a := ATT_DEF.duplicate(true)
	for k in s:
		a[k] = s[k]
	var ex: Dictionary = a.AttenuationShapeExtents
	a.algorithm = String(a.DistanceAlgorithm).get_slice("::", 1)
	a.shape = String(a.AttenuationShape).get_slice("::", 1)
	a.falloff_mode = String(a.FalloffMode).get_slice("::", 1)
	a.inner_radius_m = float(ex.get("X", 0.0)) * CM	# Sphere: Extents.X is the full-volume radius (Evaluate 0x2ef26a0)
	a.falloff_m = float(a.FalloffDistance) * CM
	a.db_at_max = float(a.dBAttenuationAtMax)
	return a

# UE AttenuationEval for a sphere, distances in metres (scale-free: d/f is the same in cm). Returns linear gain.
static func gain(a: Dictionary, dist_m: float) -> float:
	if a.is_empty() or not a.bAttenuate:
		return 1.0
	var d := maxf(dist_m - float(a.inner_radius_m), 0.0)
	var f := maxf(float(a.falloff_m), 1.0 * CM)		# max(Falloff, 1) in cm
	var r := 0.0
	match String(a.algorithm):
		"Linear": r = 1.0 - d / f
		"Logarithmic": r = -0.5 * log(maxf(d, 1e-4 * CM) / f)
		"Inverse": r = 0.02 / (maxf(d, 1e-4 * CM) / f)
		"LogReverse": r = 0.0 if d > f else 1.0 + 0.5 * log(maxf(1.0 - d / f, 1e-4))
		"NaturalSound":
			var al := d / f
			if a.falloff_mode == "Silent" and al >= 1.0:
				return 0.0
			if a.falloff_mode != "Continues":
				al = clampf(al, 0.0, 1.0)
			r = pow(10.0, al * float(a.db_at_max) / 20.0)
		_:
			push_warning("UeSound: attenuation %s not ported, using 1" % a.algorithm)
			r = 1.0
	return clampf(r, 0.0, 1.0)

static func stream(wave: String) -> AudioStream:
	if _streams.has(wave):
		return _streams[wave]
	var s: AudioStream = null
	for ext in ["ogg", "wav"]:
		var rp: String = DATA + wave + "." + ext
		if ResourceLoader.exists(rp):
			s = load(rp)
		elif FileAccess.file_exists(rp):
			s = AudioStreamOggVorbis.load_from_file(rp) if ext == "ogg" else AudioStreamWAV.load_from_file(rp)
		if s != null:
			break
	if s == null:
		push_error("UeSound: no exported audio for " + wave + " (sh scripts/mdx.sh audio)")
	_streams[wave] = s
	return s

# the SoundWave's own Volume/Pitch (ctor default 1)
static func wave_scale(wave: String) -> Vector2:
	var pr: Dictionary = UePkg.export_of(UePkg.load_pkg(wave), "SoundWave").get("Properties", {})
	return Vector2(float(pr.get("Volume", 1.0)), float(pr.get("Pitch", 1.0)))

# ---------------------------------------------------------------- evaluation (one play of the cue, like UE's ParseNodes)

# -> [{wave, volume, pitch, delay, looping}] for this play; params = Switch/Branch/ModulatorContinuous parameters
static func evaluate(d: Dictionary, params := {}, rng: RandomNumberGenerator = null) -> Array:
	if rng == null:
		rng = RandomNumberGenerator.new(); rng.randomize()
	var out := []
	_eval(d.get("root", {}), params, rng, float(d.volume), float(d.pitch), 0.0, out)
	return out

static func _eval(n: Dictionary, params: Dictionary, rng: RandomNumberGenerator, vol: float, pit: float, dly: float, out: Array) -> void:
	if n.is_empty():
		return
	var ch: Array = n.children
	match n.type:
		"WavePlayer":
			var ws := wave_scale(n.wave)
			out.append({"wave": n.wave, "volume": vol * ws.x, "pitch": pit * ws.y, "delay": dly, "looping": n.looping})
		"Modulator":
			var v: float = n.VolumeMax + (n.VolumeMin - n.VolumeMax) * rng.randf()
			var p: float = n.PitchMax + (n.PitchMin - n.PitchMax) * rng.randf()
			for c in ch: _eval(c, params, rng, vol * v, pit * p, dly, out)
		"Random":
			var i := _pick(n, rng)
			if i >= 0: _eval(ch[i], params, rng, vol, pit, dly, out)
		"Mixer":
			for i in ch.size():
				_eval(ch[i], params, rng, vol * (n.input_volume[i] if i < n.input_volume.size() else 1.0), pit, dly, out)
		"Switch":
			var i := int(params[n.param]) + 1 if params.has(n.param) else 0
			if i < 0 or i >= ch.size(): i = 0
			if i < ch.size(): _eval(ch[i], params, rng, vol, pit, dly, out)
		"Branch":
			var i := 2
			if params.has(n.param): i = 0 if bool(params[n.param]) else 1
			if i < ch.size(): _eval(ch[i], params, rng, vol, pit, dly, out)
		"Delay":
			var t := maxf(n.DelayMax + (n.DelayMin - n.DelayMax) * rng.randf(), 0.0)
			for c in ch: _eval(c, params, rng, vol, pit, dly + t, out)
		"ModulatorContinuous":
			var vv := _mc_value(n.volume_mod, params)
			var pp := _mc_value(n.pitch_mod, params)
			for c in ch: _eval(c, params, rng, vol * vv, pit * pp, dly, out)
		_:
			if not ch.is_empty(): _eval(ch[0], params, rng, vol, pit, dly, out)

# ChooseNodeIndex: weighted pick over the not-yet-used children (all, when without-replacement is off).
# Resetting the used set once every child has played is UE's documented without-replacement behaviour; the reset
# itself sits in USoundNodeRandom::ParseNodes, which was not disassembled.
static func _pick(n: Dictionary, rng: RandomNumberGenerator) -> int:
	var cnt: int = n.children.size()
	if cnt == 0:
		return -1
	if n.no_repeat and not n.used.has(false):
		n.used.fill(false)
	var tot := 0.0
	for i in cnt:
		if not (n.no_repeat and n.used[i]):
			tot += _w(n, i)
	var r := rng.randf() * tot
	var acc := 0.0
	var pick := 0
	for i in cnt:
		if n.no_repeat and n.used[i]:
			continue
		acc += _w(n, i)
		pick = i
		if r < acc:
			break
	if n.no_repeat:
		n.used[pick] = true
	return pick

static func _w(n: Dictionary, i: int) -> float:
	return n.weights[i] if i < n.weights.size() else 0.0

static func _mc_value(m: Dictionary, params: Dictionary) -> float:
	var v := float(params.get(m.param, m.Default)) if m.param != "" else float(m.Default)
	if m.mode.ends_with("Direct"):
		return v
	if m.mode.ends_with("Abs"):
		v = absf(v)
	var s := 0.0
	if m.MaxInput > m.MinInput:
		s = (m.MaxOutput - m.MinOutput) / (m.MaxInput - m.MinInput)
	return (clampf(v, m.MinInput, maxf(m.MaxInput, m.MinInput)) - m.MinInput) * s + m.MinOutput

# ---------------------------------------------------------------- Godot objects

# Cue = [Modulator...] -> Random -> WavePlayers  =>  AudioStreamRandomizer + base pitch/volume for the player.
# Modulator ranges multiply, so the bounds are prod(Min)..prod(Max). Godot's randomizer is symmetric around the
# player value (pitch in [1/r, r], volume +- offset dB), so centring the player on the geometric mean gives exactly
# UE's bounds; the distribution inside them differs (UE: uniform in linear units). Returns {} if the cue isn't that shape.
static func to_randomizer(d: Dictionary) -> Dictionary:
	var n: Dictionary = d.get("root", {})
	var pmin := 1.0; var pmax := 1.0; var vmin := 1.0; var vmax := 1.0
	while n.get("type", "") == "Modulator" and n.children.size() == 1:
		pmin *= n.PitchMin; pmax *= n.PitchMax; vmin *= n.VolumeMin; vmax *= n.VolumeMax
		n = n.children[0]
	if n.get("type", "") != "Random":
		return {}
	var r := AudioStreamRandomizer.new()
	r.playback_mode = AudioStreamRandomizer.PLAYBACK_RANDOM_NO_REPEATS if n.no_repeat else AudioStreamRandomizer.PLAYBACK_RANDOM
	for i in n.children.size():
		var c: Dictionary = n.children[i]
		if c.get("type", "") != "WavePlayer":
			return {}
		r.add_stream(-1, stream(c.wave), _w(n, i))
	var lo := pmin * float(d.pitch); var hi := pmax * float(d.pitch)
	var vlo := linear_to_db(vmin * float(d.volume)); var vhi := linear_to_db(vmax * float(d.volume))
	r.random_pitch = sqrt(hi / lo)
	r.random_volume_offset_db = (vhi - vlo) * 0.5
	return {"stream": r, "pitch_scale": sqrt(lo * hi), "volume_db": (vlo + vhi) * 0.5,
		"pitch_range": Vector2(lo, hi), "volume_range": Vector2(vmin, vmax) * float(d.volume)}

# Player settings for UE attenuation. Godot's distance models are not UE's, so Godot attenuation is switched off
# (ATTENUATION_DISABLED, max_distance 0) and UeSound multiplies UE's own curve in every frame (gain()).
static func apply_player(pl: AudioStreamPlayer3D, a: Dictionary) -> void:
	pl.attenuation_model = AudioStreamPlayer3D.ATTENUATION_DISABLED
	pl.max_distance = 0.0
	pl.attenuation_filter_cutoff_hz = 20500.0		# docs: 20500 disables Godot's distance low-pass
	if not a.is_empty() and not a.bSpatialize:
		pl.panning_strength = 0.0

# ---------------------------------------------------------------- runtime node

var desc := {}
var _live := []			# [{player, volume}]

func play_cue(params := {}) -> void:
	if not is_inside_tree():		# AudioStreamPlayer3D can only play inside the tree
		await tree_entered
		await get_tree().process_frame
	for it in evaluate(desc, params):
		var pl := AudioStreamPlayer3D.new()
		pl.stream = stream(it.wave)
		if pl.stream == null:
			pl.free(); continue
		apply_player(pl, desc.get("attenuation", {}))
		pl.pitch_scale = it.pitch
		pl.max_db = maxf(3.0, linear_to_db(it.volume))
		add_child(pl)
		var e := {"player": pl, "volume": float(it.volume)}
		_live.append(e)
		_update(e)
		pl.finished.connect(func(): _live.erase(e); pl.queue_free())
		if it.delay > 0.0:
			get_tree().create_timer(it.delay).timeout.connect(pl.play)
		else:
			pl.play()

func _process(_dt: float) -> void:
	for e in _live:
		_update(e)

func _update(e: Dictionary) -> void:
	var pl: AudioStreamPlayer3D = e.player
	if not pl.is_inside_tree():
		return
	var vp := get_viewport()
	var ln: Node3D = vp.get_audio_listener_3d()
	if ln == null:
		ln = vp.get_camera_3d()
	var g := 1.0
	if ln != null:
		g = gain(desc.get("attenuation", {}), pl.global_position.distance_to(ln.global_position))
	pl.volume_db = maxf(linear_to_db(e.volume * g), -80.0)

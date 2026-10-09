//! Map sounds: every AudioComponent placed in a map's levels (AmbientSound actors and Blueprint actors' audio
//! components: the component export of the level, its properties over its Template archetype, mh_level Pkgs::props),
//! with its world position and the PlayCue it starts. Auto-activated components start on BeginPlay
//! (UActorComponent::bAutoActivate; the AudioComponent default true is UNCONFIRMED: UAudioComponent ctor not read).

use mh_level::level::{levels, Pkgs};
use serde_json::{Map, Value};

#[derive(Clone, Debug)]
pub struct MapSound {
    pub level: String,
    /// the component export name
    pub name: String,
    /// the actor class it belongs to (the component's Outer export Type)
    pub owner: String,
    /// SoundCue / SoundWave package
    pub sound: String,
    /// world position (UE cm)
    pub pos_ue: [f64; 3],
    pub auto_activate: bool,
    pub volume: f64,
    pub pitch: f64,
    /// bOverrideAttenuation + AttenuationOverrides, else AttenuationSettings (asset), else the sound's own
    pub attenuation: Option<mh_assets::sound_cue::Attenuation>,
    pub props: Map<String, Value>,
}

fn ty(e: &Value) -> &str {
    e.get("Type").and_then(Value::as_str).unwrap_or("")
}

/// placed components of one export Type across a map's levels: (level pkg, export, merged props, world position)
pub fn placed(pk: &Pkgs, map: &str, want: &dyn Fn(&str) -> bool) -> Vec<(String, Value, Map<String, Value>, [f64; 3], String)> {
    let mut out = vec![];
    for lv in levels(pk, map) {
        let ex = pk.load_pkg(&lv.pkg);
        for e in ex.iter() {
            if !want(ty(e)) {
                continue;
            }
            let props = pk.props(e);
            let pos = (lv.xf * pk.world_xf(e)).translation();
            // the owner: the Outer's class ("AmbientSound'Level:PersistentLevel.Name'" -> "AmbientSound")
            let owner = e.get("Outer").and_then(|o| o.get("ObjectName")).and_then(Value::as_str).and_then(|o| o.split('\'').next()).unwrap_or("").to_string();
            out.push((lv.pkg.clone(), e.clone(), props, pos, owner));
        }
    }
    out
}

/// every AudioComponent with a sound in a map
pub fn map_sounds(pk: &Pkgs, map: &str) -> Vec<MapSound> {
    placed(pk, map, &|t| t == "AudioComponent")
        .into_iter()
        .filter_map(|(level, e, p, pos, owner)| {
            let sound = mh_assets::material::strip_index(&mh_assets::material::ue_pkg_path(p.get("Sound")?)).to_string();
            if sound.is_empty() {
                return None;
            }
            let b = |k: &str, d: bool| p.get(k).and_then(Value::as_bool).unwrap_or(d);
            let f = |k: &str, d: f64| p.get(k).and_then(Value::as_f64).unwrap_or(d);
            let attenuation = if b("bOverrideAttenuation", false) {
                Some(mh_assets::sound_cue::attenuation_settings(p.get("AttenuationOverrides").unwrap_or(&Value::Null)))
            } else {
                p.get("AttenuationSettings").and_then(|a| a.get("ObjectPath")).and_then(Value::as_str).and_then(|a| mh_assets::sound_cue::attenuation(&pk.rd, a))
            };
            Some(MapSound {
                level,
                name: e.get("Name").and_then(Value::as_str).unwrap_or("").to_string(),
                owner,
                sound,
                pos_ue: pos,
                auto_activate: b("bAutoActivate", true),
                volume: f("VolumeMultiplier", 1.0),
                pitch: f("PitchMultiplier", 1.0),
                attenuation,
                props: p,
            })
        })
        .collect()
}

/// an AudioVolume: priority, reverb and ambient zone settings, and its brush (convex elements' boxes, world UE cm)
#[derive(Clone, Debug)]
pub struct AudioVol {
    pub name: String,
    pub priority: f64,
    pub enabled: bool,
    /// Settings.ReverbEffect (ReverbEffect package; empty = none) and Settings.Volume / FadeTime
    pub reverb: String,
    pub reverb_volume: f64,
    pub reverb_fade: f64,
    /// Settings.bApplyReverb (FReverbSettings default true)
    pub apply_reverb: bool,
    /// AmbientZoneSettings as stored (None = the defaults: no interior / exterior change)
    pub ambient_zone: Option<Value>,
    pub boxes: Vec<([f64; 3], [f64; 3])>,
}

/// the map's AudioVolumes (mh-level LevelData volumes + the actor's properties). Defaults where a volume stores
/// nothing: Priority 0, bEnabled true, reverb Volume 0.5, FadeTime 2 (FReverbSettings; UNCONFIRMED: ctor not read).
pub fn audio_volumes(pk: &Pkgs, d: &mh_level::level::LevelData) -> Vec<AudioVol> {
    d.volumes
        .iter()
        .filter(|v| v.class == "AudioVolume")
        .map(|v| {
            let p = pk.obj(Some(&serde_json::json!({ "ObjectPath": v.actor_path }))).map(|o| pk.props(&o)).unwrap_or_default();
            let st = p.get("Settings");
            let boxes = v
                .elems
                .iter()
                .filter(|e| !e.is_empty())
                .map(|e| {
                    let mut lo = [f64::MAX; 3];
                    let mut hi = [f64::MIN; 3];
                    for q in e {
                        for i in 0..3 {
                            lo[i] = lo[i].min(q[i]);
                            hi[i] = hi[i].max(q[i]);
                        }
                    }
                    (lo, hi)
                })
                .collect();
            AudioVol {
                name: v.name.clone(),
                priority: p.get("Priority").and_then(Value::as_f64).unwrap_or(0.0),
                enabled: p.get("bEnabled").and_then(Value::as_bool).unwrap_or(true),
                reverb: st.and_then(|s| s.get("ReverbEffect")).map(|r| mh_assets::material::strip_index(&mh_assets::material::ue_pkg_path(r)).to_string()).unwrap_or_default(),
                reverb_volume: st.and_then(|s| s.get("Volume")).and_then(Value::as_f64).unwrap_or(0.5),
                reverb_fade: st.and_then(|s| s.get("FadeTime")).and_then(Value::as_f64).unwrap_or(2.0),
                apply_reverb: st.and_then(|s| s.get("bApplyReverb")).and_then(Value::as_bool).unwrap_or(true),
                ambient_zone: p.get("AmbientZoneSettings").cloned(),
                boxes: if v.elems.is_empty() { vec![(v.min, v.max)] } else { boxes },
            }
        })
        .collect()
}

/// the enabled volume of highest priority containing a point (UE cm). Containment uses the boxes of the brush's
/// convex elements (UNCONFIRMED approximation of the brush's encompassing test)
pub fn volume_at(vols: &[AudioVol], p: [f64; 3]) -> Option<&AudioVol> {
    vols.iter()
        .filter(|v| v.enabled && v.boxes.iter().any(|(lo, hi)| (0..3).all(|i| p[i] >= lo[i] && p[i] <= hi[i])))
        .max_by(|a, b| a.priority.total_cmp(&b.priority))
}

/// a placed BP_AmbientRandomAudioSpawner (Audio/Cues/Ambient/Blueprints): its variables over the class defaults
#[derive(Clone, Debug)]
pub struct RandomSpawner {
    pub name: String,
    pub sound: String,
    /// the actor's root (UE cm)
    pub pos_ue: [f64; 3],
    pub time_min: f64,
    pub time_max: f64,
    pub distance: f64,
    pub max_height: f64,
}

pub const RANDOM_SPAWNER: &str = "Mordhau/Content/Mordhau/Audio/Cues/Ambient/Blueprints/BP_AmbientRandomAudioSpawner";

/// every random audio spawner of a map (actor exports of class BP_AmbientRandomAudioSpawner_C; variables the
/// instance does not store take the class defaults, a variable the CDO does not store is 0)
pub fn random_spawners(pk: &Pkgs, map: &str) -> Vec<RandomSpawner> {
    let d = pk.defaults(RANDOM_SPAWNER);
    let mut out = vec![];
    for lv in levels(pk, map) {
        let ex = pk.load_pkg(&lv.pkg);
        for e in ex.iter() {
            if ty(e) != "BP_AmbientRandomAudioSpawner_C" {
                continue;
            }
            let p = pk.props(e);
            let g = |k: &str| p.get(k).or_else(|| d.get(k)).and_then(Value::as_f64).unwrap_or(0.0);
            let sound = mh_assets::material::strip_index(&mh_assets::material::ue_pkg_path(p.get("Sound").or_else(|| d.get("Sound")).unwrap_or(&Value::Null))).to_string();
            let root = pk.obj(p.get("RootComponent"));
            let pos = root.map(|r| (lv.xf * pk.world_xf(&r)).translation()).unwrap_or([0.0; 3]);
            out.push(RandomSpawner {
                name: e.get("Name").and_then(Value::as_str).unwrap_or("").to_string(),
                sound,
                pos_ue: pos,
                time_min: g("Time_Min"),
                time_max: g("Time_Max"),
                distance: g("Distance"),
                max_height: g("MaxHeight"),
            });
        }
    }
    out
}

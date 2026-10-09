//! ue.rs - package JSON access and the UE -> glTF placement rules. Pure functions, no Bevy systems.
//!
//! Port of godot/components/ue/pkg/ue_pkg.gd (load_pkg/strip) and godot/components/ue/ue_level.gd (obj/props/levels/
//! rot_quat/quat/xf/rel_xf/ftransform/world_xf/read). The comments of ue_level.gd's header carry the sources:
//! CUE4Parse Gltf.cs:25 (UnitScale 0.01), 72/230 (positions), 244-248 (SwapYZ), 250 (quaternion (x, z, y, -w)),
//! FRotator.Quaternion (FRotator.cs:88-111), FTransform.TransformPosition (FTransform.cs:389).
//! Package data comes from extract/json (`mdx json`, the Godot `json` backend); R4 switches to mh-pak.

use bevy::math::{Affine3A, Mat3, Quat, Vec3};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// "Mordhau/Content/.../BP_X.0" -> "Mordhau/Content/.../BP_X" (mdx json appends the export index). ue_pkg.gd strip.
pub fn strip(obj_path: &str) -> &str {
    match obj_path.rsplit_once('.') {
        Some((base, ext)) if !ext.is_empty() && ext.parse::<i64>().is_ok() => base,
        _ => obj_path,
    }
}

/// Export index of "pkg.N", if any.
pub fn obj_index(obj_path: &str) -> Option<usize> {
    obj_path.rsplit_once('.').and_then(|(_, e)| e.parse::<usize>().ok())
}

/// "/Game/Mordhau/Maps/X/Y.Y" -> "Mordhau/Content/Mordhau/Maps/X/Y". ue_level.gd game_pkg.
pub fn game_pkg(asset_path: &str) -> String {
    let file = asset_path.rsplit('/').next().unwrap_or("");
    let p = if file.contains('.') { asset_path.rsplit_once('.').map(|x| x.0).unwrap_or(asset_path) } else { asset_path };
    format!("Mordhau/Content/{}", p.trim_start_matches("/Game/"))
}

/// Package JSON cache over extract/json (UePkg.load_pkg).
pub struct Pkgs {
    pub json_root: PathBuf,
    cache: HashMap<String, Arc<Vec<Value>>>,
    pub missing: Vec<String>,
}

impl Pkgs {
    pub fn new(json_root: impl AsRef<Path>) -> Self {
        Pkgs { json_root: json_root.as_ref().to_path_buf(), cache: HashMap::new(), missing: Vec::new() }
    }

    pub fn exists(&self, pkg: &str) -> bool {
        self.json_root.join(format!("{}.json", strip(pkg))).is_file()
    }

    pub fn load_pkg(&mut self, obj_path: &str) -> Arc<Vec<Value>> {
        let p = strip(obj_path).to_string();
        if let Some(a) = self.cache.get(&p) {
            return a.clone();
        }
        let path = self.json_root.join(format!("{p}.json"));
        let a: Vec<Value> = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
            .and_then(|v| if let Value::Array(a) = v { Some(a) } else { None })
            .unwrap_or_default();
        if a.is_empty() {
            self.missing.push(p.clone());
        }
        let a = Arc::new(a);
        self.cache.insert(p, a.clone());
        a
    }

    /// {"ObjectPath": "pkg.N"} -> export N of pkg (ue_level.gd obj).
    pub fn obj(&mut self, r: Option<&Value>) -> Option<Value> {
        let op = r?.get("ObjectPath")?.as_str()?;
        let i = obj_index(op)?;
        self.load_pkg(op).get(i).cloned()
    }

    /// Own properties over the Template's, recursively (ue_level.gd props).
    pub fn props(&mut self, e: &Value) -> serde_json::Map<String, Value> {
        let own = e.get("Properties").and_then(|p| p.as_object()).cloned().unwrap_or_default();
        let Some(t) = e.get("Template") else { return own };
        let Some(base_e) = self.obj(Some(t)) else { return own };
        let mut out = self.props(&base_e);
        for (k, v) in own {
            out.insert(k, v);
        }
        out
    }

    /// Persistent map + every streaming sub-level, depth first: (package, level transform). ue_level.gd levels.
    pub fn levels(&mut self, map_pkg: &str, xf: Affine3A, out: &mut Vec<(String, Affine3A)>) {
        out.push((map_pkg.to_string(), xf));
        let exps = self.load_pkg(map_pkg);
        for e in exps.iter() {
            if !e.get("Type").and_then(|t| t.as_str()).unwrap_or("").starts_with("LevelStreaming") {
                continue;
            }
            let p = e.get("Properties");
            let wa = p.and_then(|p| p.get("WorldAsset")).and_then(|w| w.get("AssetPathName")).and_then(|s| s.as_str()).unwrap_or("");
            if wa.is_empty() {
                continue;
            }
            let lx = xf * p.and_then(|p| p.get("LevelTransform")).map(ftransform).unwrap_or(Affine3A::IDENTITY);
            self.levels(&game_pkg(wa), lx, out);
        }
    }

    /// Component -> world, up the AttachParent chain (ue_level.gd world_xf).
    pub fn world_xf(&mut self, e: &Value) -> Affine3A {
        let p = self.props(e);
        let local = rel_xf(&p);
        match self.obj(p.get("AttachParent")) {
            Some(par) => self.world_xf(&par) * local,
            None => local,
        }
    }
}

fn f(v: Option<&Value>, k: &str, d: f32) -> f32 {
    v.and_then(|v| v.get(k)).and_then(|x| x.as_f64()).map(|x| x as f32).unwrap_or(d)
}

/// UE (X, Y, Z) -> glTF/Bevy (X, Z, Y) (Gltf.cs:244-248 SwapYZ).
pub fn swap(v: Option<&Value>) -> Vec3 {
    Vec3::new(f(v, "X", 0.0), f(v, "Z", 0.0), f(v, "Y", 0.0))
}

/// CUE4Parse FRotator.Quaternion (FRotator.cs:88-111), degrees in, UE quaternion (x, y, z, w) out.
pub fn rot_quat(r: Option<&Value>) -> [f32; 4] {
    let h = std::f32::consts::PI / 360.0;
    let (p, y, o) = (f(r, "Pitch", 0.0) * h, f(r, "Yaw", 0.0) * h, f(r, "Roll", 0.0) * h);
    let (sp, cp, sy, cy, sr, cr) = (p.sin(), p.cos(), y.sin(), y.cos(), o.sin(), o.cos());
    [
        cr * sp * sy - sr * cp * cy,
        -cr * sp * cy - sr * cp * sy,
        cr * cp * sy - sr * sp * cy,
        cr * cp * cy + sr * sp * sy,
    ]
}

/// UE quaternion (x, y, z, w) -> glTF space via the writer's SwapYZ(FQuat) = (x, z, y, -w) (Gltf.cs:250).
pub fn quat(q: [f32; 4]) -> Quat {
    Quat::from_xyzw(q[0], q[2], q[1], -q[3]).normalize()
}

/// origin = SwapYZ(T) * 0.01, basis = R(SwapYZ(q)) * scale(sx, sz, sy)  (ue_level.gd header / xf).
pub fn xf(loc: Option<&Value>, rot: [f32; 4], scl: Option<&Value>) -> Affine3A {
    let s = Vec3::new(f(scl, "X", 1.0), f(scl, "Z", 1.0), f(scl, "Y", 1.0));
    let m = Mat3::from_quat(quat(rot)) * Mat3::from_diagonal(s);
    Affine3A::from_mat3_translation(m, swap(loc) * 0.01)
}

/// Relative transform of a SceneComponent; absent = USceneComponent defaults (zero loc/rot, unit scale). rel_xf.
pub fn rel_xf(p: &serde_json::Map<String, Value>) -> Affine3A {
    xf(p.get("RelativeLocation"), rot_quat(p.get("RelativeRotation")), p.get("RelativeScale3D"))
}

/// FTransform JSON {Rotation{X,Y,Z,W}, Translation, Scale3D} (foliage TransformData, LevelTransform). ftransform.
pub fn ftransform(t: &Value) -> Affine3A {
    let r = t.get("Rotation");
    xf(t.get("Translation"), [f(r, "X", 0.0), f(r, "Y", 0.0), f(r, "Z", 0.0), f(r, "W", 1.0)], t.get("Scale3D"))
}

// ---------------------------------------------------------------- level read (ue_level.gd read)

#[derive(Clone, Debug)]
#[allow(dead_code)] // actor: R2 HLOD sub-actor switching
pub struct MeshRec {
    pub name: String,
    pub actor: String,
    /// StaticMesh object path ("pkg.N")
    pub mesh: String,
    pub xf: Affine3A,
    /// OverrideMaterials, per slot: material package or "" (keep the mesh's own)
    pub mats: Vec<String>,
    pub noshadow: bool,
    /// instanced components: one transform per PerInstanceSMData entry (component space)
    pub inst: Vec<Affine3A>,
}

#[derive(Clone, Debug)]
pub struct StartRec {
    pub name: String,
    pub xf: Affine3A,
    pub team: Option<i64>,
}

#[derive(Default, Debug)]
pub struct LevelInfo {
    pub levels: Vec<String>,
    pub meshes: Vec<MeshRec>,
    pub isms: Vec<MeshRec>,
    pub skips: BTreeMap<String, Vec<String>>,
    pub starts: Vec<StartRec>,
    pub sun: Option<Affine3A>,
    pub seen: usize,
}

fn ty(e: &Value) -> &str {
    e.get("Type").and_then(|t| t.as_str()).unwrap_or("")
}

fn is_mesh_comp(e: &Value) -> bool {
    let t = ty(e);
    t.ends_with("StaticMeshComponent") || t == "SplineMeshComponent" || e.get("PerInstanceSMData").is_some()
}

/// map_pkg: "Mordhau/Content/Mordhau/Maps/Arena_Map/DU_Arena". Port of UeLevel.read minus lightmaps / HLOD proxies /
/// crowd (R2). HLOD components are skipped (counted "hlod") exactly as the Godot reader skips them from the placed set.
pub fn read_level(pk: &mut Pkgs, map_pkg: &str) -> LevelInfo {
    let mut info = LevelInfo::default();
    let mut lvs = Vec::new();
    pk.levels(map_pkg, Affine3A::IDENTITY, &mut lvs);
    for (pkg, lx) in lvs {
        info.levels.push(pkg.clone());
        let exps = pk.load_pkg(&pkg);
        for e in exps.iter() {
            let t = ty(e);
            if t == "MordhauPlayerStart" || t == "PlayerStart" || (t.ends_with("PlayerStart_C") && t.starts_with("BP_")) {
                let rc = pk.obj(e.get("Properties").and_then(|p| p.get("RootComponent")));
                let xf = lx * rc.map(|rc| pk.world_xf(&rc)).unwrap_or(Affine3A::IDENTITY);
                // Team: the instance value (Blueprint class defaults not read in R1; UNCONFIRMED for BP_*PlayerStart_C)
                let team = e.get("Properties").and_then(|p| p.get("Team")).and_then(|v| v.as_i64());
                info.starts.push(StartRec { name: e.get("Name").and_then(|n| n.as_str()).unwrap_or("").into(), xf, team });
            } else if t == "DirectionalLight" && info.sun.is_none() {
                let rc = pk.obj(e.get("Properties").and_then(|p| p.get("RootComponent")));
                info.sun = Some(lx * rc.map(|rc| pk.world_xf(&rc)).unwrap_or(Affine3A::IDENTITY));
            }
            if !is_mesh_comp(e) {
                continue;
            }
            info.seen += 1;
            let actor = pk.obj(e.get("Outer")).unwrap_or(Value::Null);
            let actor_name = actor.get("Name").and_then(|n| n.as_str()).unwrap_or("?").to_string();
            let name = format!("{}.{}", actor_name, e.get("Name").and_then(|n| n.as_str()).unwrap_or("?"));
            let p = pk.props(e);
            let mesh = p.get("StaticMesh").and_then(|m| m.get("ObjectPath")).and_then(|s| s.as_str()).unwrap_or("").to_string();
            let mut skip = |why: &str| info.skips.entry(why.to_string()).or_default().push(name.clone());
            if ty(&actor) == "LODActor" {
                skip("hlod");
                continue;
            }
            if actor.get("Properties").and_then(|p| p.get("bHidden")).and_then(|b| b.as_bool()).unwrap_or(false) {
                skip("hidden");
                continue;
            }
            if t == "SplineMeshComponent" {
                skip("spline");
                continue;
            }
            if mesh.is_empty() {
                skip("no_mesh");
                continue;
            }
            let mp = strip(&mesh);
            if mp == "Engine/Content/EngineSky/SM_SkySphere" || mp.starts_with("Mordhau/Content/UltraDynamicSky/Meshes/") {
                skip("sky");
                continue;
            }
            let mats = p
                .get("OverrideMaterials")
                .and_then(|m| m.as_array())
                .map(|a| a.iter().map(|m| m.get("ObjectPath").and_then(|s| s.as_str()).map(|s| strip(s).to_string()).unwrap_or_default()).collect())
                .unwrap_or_default();
            // CastShadow absent = mesh-component default true (ue_level.gd; UNCONFIRMED there too)
            let noshadow = p.get("CastShadow").and_then(|b| b.as_bool()) == Some(false);
            let xf = lx * pk.world_xf(e);
            let mut rec = MeshRec { name: name.clone(), actor: actor_name, mesh, xf, mats, noshadow, inst: Vec::new() };
            if e.get("PerInstanceSMData").is_some() || t.contains("Instanced") {
                rec.inst = e
                    .get("PerInstanceSMData")
                    .and_then(|a| a.as_array())
                    .map(|a| a.iter().map(|d| d.get("TransformData").map(ftransform).unwrap_or(Affine3A::IDENTITY)).collect())
                    .unwrap_or_default();
                if rec.inst.is_empty() {
                    info.skips.entry("empty".into()).or_default().push(name);
                    continue;
                }
                info.isms.push(rec);
            } else {
                info.meshes.push(rec);
            }
        }
    }
    info
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strip_and_game_pkg() {
        assert_eq!(strip("Mordhau/Content/A/B.14"), "Mordhau/Content/A/B");
        assert_eq!(strip("Mordhau/Content/A/B"), "Mordhau/Content/A/B");
        assert_eq!(game_pkg("/Game/Mordhau/Maps/Arena_Map/Arena.Arena"), "Mordhau/Content/Mordhau/Maps/Arena_Map/Arena");
    }

    /// A pure yaw of 90 degrees turns UE +X into UE +Y; in glTF space that is (1,0,0) -> (0,0,1) (SwapYZ of +Y).
    #[test]
    fn yaw_maps_through_swap() {
        let q = quat(rot_quat(Some(&json!({"Yaw": 90.0}))));
        let v = q * Vec3::X;
        assert!((v - Vec3::Z).length() < 1e-5, "{v:?}");
    }

    /// Same counts as godot/tests/test_level.gd test_level_counts / test_level_sublevels on extract/json (830 mesh
    /// components = 14 DU_Arena + 806 Arena + 10 foliage; skips hlod 61, hidden 1, sky 1, empty 1). Skipped when
    /// extract/ is absent (game data is never in the repo).
    #[test]
    fn du_arena_matches_godot_reader() {
        let json = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../extract/json");
        if !json.is_dir() {
            eprintln!("skip: no extract/json");
            return;
        }
        let mut pk = Pkgs::new(&json);
        let i = read_level(&mut pk, "Mordhau/Content/Mordhau/Maps/Arena_Map/DU_Arena");
        assert_eq!(i.levels, ["Mordhau/Content/Mordhau/Maps/Arena_Map/DU_Arena", "Mordhau/Content/Mordhau/Maps/Arena_Map/Arena"]);
        assert_eq!(i.seen, 830);
        let sk: BTreeMap<_, _> = i.skips.iter().map(|(k, v)| (k.as_str(), v.len())).collect();
        assert_eq!(sk, BTreeMap::from([("empty", 1), ("hidden", 1), ("hlod", 61), ("sky", 1)]));
        assert_eq!(i.meshes.len() + i.isms.len(), 830 - 64);
        let mut teams: Vec<_> = i.starts.iter().filter(|s| s.name.starts_with("Spawn")).map(|s| s.team).collect();
        teams.sort();
        assert_eq!(teams, [Some(0), Some(1)]);
        // SpawnBlue: CollisionCapsule at UE (1345, 0, 167) -> glTF (13.45, 1.67, 0)
        let blue = i.starts.iter().find(|s| s.name == "SpawnBlue").unwrap();
        assert!((Vec3::from(blue.xf.translation) - Vec3::new(13.45, 1.67, 0.0)).length() < 1e-4);
        assert!(i.sun.is_some());
    }

    /// Placement equals SwapYZ(T + R(s * v)) * 0.01 for a UE vertex v (the rule test_level.gd proves in Godot).
    #[test]
    fn placement_matches_ue_transform() {
        let loc = json!({"X": 100.0, "Y": 200.0, "Z": 300.0});
        let rot = json!({"Pitch": 30.0, "Yaw": 45.0, "Roll": 10.0});
        let scl = json!({"X": 2.0, "Y": 3.0, "Z": 4.0});
        let a = xf(Some(&loc), rot_quat(Some(&rot)), Some(&scl));
        let v_ue = Vec3::new(5.0, -7.0, 11.0); // cm
        // UE side: rotate the scaled vertex with the UE quaternion (left-handed Z-up frame, but quaternion math is
        // the same Hamilton product), translate, then convert.
        let q = rot_quat(Some(&rot));
        let qu = Quat::from_xyzw(q[0], q[1], q[2], q[3]);
        let w_ue = qu * (v_ue * Vec3::new(2.0, 3.0, 4.0)) + Vec3::new(100.0, 200.0, 300.0);
        let want = Vec3::new(w_ue.x, w_ue.z, w_ue.y) * 0.01;
        // glb vertex of v_ue is SwapYZ(v * 0.01)
        let g = Vec3::new(v_ue.x, v_ue.z, v_ue.y) * 0.01;
        let got = a.transform_point3(g);
        assert!((got - want).length() < 1e-4, "got {got:?} want {want:?}");
    }
}

//! A UE level (persistent map + streaming sub-levels) read from the paks into engine-neutral placement data.
//! Port of `components/ue/ue_level.gd` (`levels`, `props`, `world_xf`, `read`, `hlod_record`, `crowd`), the lighting
//! read of `ue_light.gd` (`read`), `records/mode_data.gd` `nav_bounds` and `ue_uds.gd` `level_of`. The package data is
//! mh-pak's export JSON (the same values as extract/json, `mh-pak` equivalence tests), so the rules port one to one.
//!
//!   map package -> World.StreamingLevels -> LevelStreaming*.WorldAsset (recursively, LevelTransform composed)
//!   every placed StaticMeshComponent           -> one MeshPlacement
//!   every (Foliage)InstancedStaticMeshComponent -> one MeshPlacement with `instances` (PerInstanceSMData)
//!   component world transform = level transform * AttachParent chain * RelativeLocation/Rotation/Scale3D
//!
//! Object references are {"ObjectPath": "<package>.<N>"}: N is the index into that package's export list.
//! All transforms are UE space (cm, Z up); `Xf::to_gltf` gives the exported meshes' space (metres, Y up).

use crate::xf::{ftransform, rel_xf, vec3, Xf, IDENTITY};
use mh_pak::{pkg as mpkg, Reader};
use serde_json::{Map, Value};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

/// An export of a package: the package's exports + the index
#[derive(Clone)]
pub struct Obj(pub Rc<Vec<Value>>, pub usize);
impl std::ops::Deref for Obj {
    type Target = Value;
    fn deref(&self) -> &Value {
        &self.0[self.1]
    }
}

/// "/Game/Mordhau/Maps/X/Y.Y" -> "Mordhau/Content/Mordhau/Maps/X/Y" (UeLevel.game_pkg)
pub fn game_pkg(asset_path: &str) -> String {
    let file = asset_path.rsplit('/').next().unwrap_or("");
    let p = if file.contains('.') { &asset_path[..asset_path.rfind('.').unwrap()] } else { asset_path };
    format!("Mordhau/Content/{}", p.strip_prefix("/Game/").unwrap_or(p))
}

/// "pkg.N" -> "pkg" (UeLevel.pkg_of)
pub fn pkg_of(obj_path: &str) -> &str {
    mh_pak::reader::strip_index(obj_path)
}

fn s<'a>(v: &'a Value, k: &str) -> &'a str {
    v.get(k).and_then(|x| x.as_str()).unwrap_or("")
}
fn ty(e: &Value) -> &str {
    s(e, "Type")
}
fn props_of(e: &Value) -> &Map<String, Value> {
    static EMPTY: std::sync::OnceLock<Map<String, Value>> = std::sync::OnceLock::new();
    e.get("Properties").and_then(|p| p.as_object()).unwrap_or_else(|| EMPTY.get_or_init(Map::new))
}
fn obj_path(r: Option<&Value>) -> &str {
    r.and_then(|r| r.get("ObjectPath")).and_then(|v| v.as_str()).unwrap_or("")
}

/// Package exports, cached per map read (UePkg.load_pkg over the pak backend)
pub struct Pkgs {
    pub rd: Reader,
    cache: RefCell<HashMap<String, Rc<Vec<Value>>>>,
    defaults: RefCell<HashMap<String, Rc<Map<String, Value>>>>,
}

impl Pkgs {
    pub fn new(rd: Reader) -> Pkgs {
        Pkgs { rd, cache: RefCell::new(HashMap::new()), defaults: RefCell::new(HashMap::new()) }
    }
    pub fn clear(&self) {
        self.cache.borrow_mut().clear();
        self.defaults.borrow_mut().clear();
        self.rd.clear_cache();
    }
    pub fn load_pkg(&self, pkg: &str) -> Rc<Vec<Value>> {
        let p = pkg_of(pkg);
        let k = p.to_lowercase();
        if let Some(v) = self.cache.borrow().get(&k) {
            return v.clone();
        }
        let v = Rc::new(self.rd.read(p).unwrap_or_default());
        self.cache.borrow_mut().insert(k, v.clone());
        v
    }
    /// {"ObjectPath": "pkg.N"} -> export N of pkg (UeLevel.obj)
    pub fn obj(&self, r: Option<&Value>) -> Option<Obj> {
        let op = obj_path(r);
        let (p, i) = op.rsplit_once('.')?;
        let i: usize = i.parse().ok()?;
        let a = self.load_pkg(p);
        (i < a.len()).then(|| Obj(a, i))
    }
    /// Own properties over the Template's (the archetype a Blueprint component was copied from), recursively
    /// (UeLevel.props)
    pub fn props(&self, e: &Value) -> Map<String, Value> {
        let own = props_of(e);
        let Some(t) = e.get("Template") else { return own.clone() };
        // a Blueprint component template's own Template link already leads through the parent classes' templates,
        // InheritableComponentHandler overrides included (checked on every map: tests/hidden.rs template_chain_changes)
        let mut out = match self.obj(Some(t)) {
            Some(o) => self.props(&o),
            None => Map::new(),
        };
        for (k, v) in own {
            out.insert(k.clone(), v.clone());
        }
        out
    }
    /// Component -> level space, up the AttachParent chain (UeLevel.world_xf). Plain affine products, as Godot's
    /// Transform3D products (UE's FTransform product drops shear under non-uniform parent scale; no chain in the maps
    /// has a scaled parent, test_level.gd)
    pub fn world_xf(&self, e: &Value) -> Xf {
        let mut x = IDENTITY;
        let mut cur: Option<Obj> = None;
        let mut first = Some(e);
        for _ in 0..64 {
            let p = match (&first, &cur) {
                (Some(e), _) => self.props(e),
                (None, Some(o)) => self.props(o),
                _ => break,
            };
            first = None;
            x = rel_xf(&p) * x;
            cur = self.obj(p.get("AttachParent"));
            if cur.is_none() {
                break;
            }
        }
        x
    }
    /// Class defaults merged over the Blueprint chain (UePkg.defaults), cached
    pub fn defaults(&self, class_pkg: &str) -> Rc<Map<String, Value>> {
        if let Some(d) = self.defaults.borrow().get(class_pkg) {
            return d.clone();
        }
        let d = Rc::new(self.rd.defaults(class_pkg));
        self.defaults.borrow_mut().insert(class_pkg.to_string(), d.clone());
        d
    }
}

/// A component is not drawn in game when (merged instance over the template / archetype chain) bHiddenInGame is true
/// or bVisible is false (UPrimitiveComponent::ShouldRender / USceneComponent visibility, UE 4.26), or its owning
/// actor is hidden (AActor bHidden over the class defaults) or editor-only (AActor bIsEditorOnlyActor,
/// UActorComponent bIsEditorOnly: not cooked into the game, but a cooked one is dropped at load)
pub fn hidden_in_game(pk: &Pkgs, e: &Value, actor: Option<&Value>) -> bool {
    // the merged instance over its Template chain (`Pkgs::props`: the cooked <Name>_GEN_VARIABLE templates link to
    // the parent class's template, InheritableComponentHandler overrides included)
    let p = pk.props(e);
    let b = |m: &Map<String, Value>, k: &str| m.get(k).and_then(|v| v.as_bool());
    if b(&p, "bHiddenInGame") == Some(true) || b(&p, "bVisible") == Some(false) || b(&p, "bIsEditorOnly") == Some(true) {
        return true;
    }
    if let Some(a) = actor {
        let mut ap = if ty(a).ends_with("_C") { (*pk.defaults(&class_pkg(a))).clone() } else { Map::new() };
        for (k, v) in props_of(a) {
            ap.insert(k.clone(), v.clone());
        }
        if b(&ap, "bHidden") == Some(true) || b(&ap, "bIsEditorOnlyActor") == Some(true) {
            return true;
        }
    }
    false
}

/// The class package of a Blueprint instance export ("BlueprintGeneratedClass'Pkg/BP_X.BP_X_C'" -> "Pkg/BP_X")
/// (UeLevel.class_pkg)
pub fn class_pkg(e: &Value) -> String {
    let c = s(e, "Class");
    let (Some(q), Some(r)) = (c.find('\''), c.rfind('\'')) else { return String::new() };
    if r <= q {
        return String::new();
    }
    let inner = &c[q + 1..r];
    let file_start = inner.rfind('/').map_or(0, |i| i + 1);
    match inner[file_start..].rfind('.') {
        Some(d) => inner[..file_start + d].to_string(),
        None => inner.to_string(),
    }
}

// ---------------------------------------------------------------- data

/// One level of the map: package and its transform (identity for the persistent level)
#[derive(Clone, Debug)]
pub struct LevelRef {
    pub pkg: String,
    pub xf: Xf,
}

/// An actor placed in a level (an export whose Outer is that level's PersistentLevel)
#[derive(Clone, Debug)]
pub struct Actor {
    pub level: usize,
    pub name: String,
    /// export Type: native class name or Blueprint "BP_X_C"
    pub class: String,
    /// world transform of its RootComponent (None when it has none)
    pub xf: Option<Xf>,
    pub hidden: bool,
}

/// A placed static mesh (StaticMeshComponent) or an instanced one (instances non-empty)
#[derive(Clone, Debug)]
pub struct MeshPlacement {
    pub level: usize,
    /// "<actor>.<component>"
    pub name: String,
    pub actor: String,
    /// export Type of the component
    pub component: String,
    /// StaticMesh object path "pkg.N"
    pub mesh: String,
    /// component world transform (UE space)
    pub xf: Xf,
    /// OverrideMaterials per material slot: material package, "" = the mesh's own slot material
    pub materials: Vec<String>,
    /// UPrimitiveComponent CastShadow (absent = true)
    pub cast_shadow: bool,
    /// OverrideMaterials as object paths ("pkg.N"): a MaterialInstanceDynamic lives in the level package itself
    pub material_paths: Vec<String>,
    /// PerInstanceSMData transforms, relative to `xf` (instanced components only)
    pub instances: Vec<Xf>,
    /// native LODData: MapBuildDataId (lightmap / shadowmap key into `LevelData::build`) and painted vertex colours
    /// (OverrideVertexColors, ue_vcolor.gd) per LOD
    pub lods: Vec<crate::native::LodInfo>,
    /// the component export ("<level package>.<export index>"), for its BodyInstance / collision settings
    pub component_path: String,
    /// not drawn in game (`hidden_in_game`): still collides
    pub hidden_in_game: bool,
}
impl MeshPlacement {
    pub fn mesh_pkg(&self) -> &str {
        pkg_of(&self.mesh)
    }
    pub fn is_instanced(&self) -> bool {
        !self.instances.is_empty()
    }
}

/// A SplineMeshComponent: the static mesh bent along a cubic Hermite segment (USplineMeshComponent; params
/// FSplineMeshParams, CUE4Parse FSplineMeshParams.cs:12-56 with its defaults; deformation CalcSliceTransform,
/// USplineMeshComponent.cs:99-225). All vectors in component space, UE cm; rolls in radians.
#[derive(Clone, Debug)]
pub struct SplineMeshPlacement {
    pub level: usize,
    pub name: String,
    /// the component export "pkg.N" (collision settings)
    pub component_path: String,
    pub actor: String,
    pub mesh: String,
    pub xf: Xf,
    pub materials: Vec<String>,
    pub material_paths: Vec<String>,
    pub cast_shadow: bool,
    pub start_pos: [f64; 3],
    pub start_tangent: [f64; 3],
    pub start_scale: [f64; 2],
    pub start_roll: f64,
    pub start_offset: [f64; 2],
    pub end_pos: [f64; 3],
    pub end_tangent: [f64; 3],
    pub end_scale: [f64; 2],
    pub end_roll: f64,
    pub end_offset: [f64; 2],
    /// ESplineMeshAxis: 0 = X, 1 = Y, 2 = Z (default X)
    pub forward_axis: u8,
    pub spline_up_dir: [f64; 3],
    pub spline_boundary_min: f64,
    pub spline_boundary_max: f64,
    pub smooth_interp_roll_scale: bool,
    pub lods: Vec<crate::native::LodInfo>,
}

impl SplineMeshPlacement {
    /// Mesh-space point -> component space, bent along the spline: USplineMeshComponent::CalcSliceTransform
    /// (CUE4Parse USplineMeshComponent.cs:99-225; FSplineMeshParams SplineEvalPos / SplineEvalTangent, cubic Hermite,
    /// FSplineMeshParams.cs:56-90). `bounds` = the mesh's (min, max) along the forward axis (render bounds); a custom
    /// SplineBoundaryMin/Max replaces them. The point's forward coordinate selects the slice and is zeroed before the
    /// slice transform applies.
    pub fn deform(&self, p: [f64; 3], bounds: (f64, f64)) -> [f64; 3] {
        let ax = self.forward_axis as usize;
        let custom = (self.spline_boundary_min - self.spline_boundary_max).abs() > 1e-8;
        let (b0, b1) = if custom { (self.spline_boundary_min, self.spline_boundary_max) } else { bounds };
        let alpha = if b1 - b0 > 1e-8 { (p[ax] - b0) / (b1 - b0) } else { 0.0 };
        // ComputeVisualMeshSplineTRange: with a custom boundary the mesh bounds' T range, extrapolation capped at 4
        let (min_t, max_t) = if custom {
            ((-4f64).max((bounds.0 - b0) / (b1 - b0)), ((bounds.1 - b0) / (b1 - b0)).min(4.0))
        } else {
            (0.0, 1.0)
        };
        let (sp, st, ep, et) = (self.start_pos, self.start_tangent, self.end_pos, self.end_tangent);
        let pos = |a: f64| -> [f64; 3] {
            let (a2, a3) = (a * a, a * a * a);
            let (h0, h1, h2, h3) = (2.0 * a3 - 3.0 * a2 + 1.0, a3 - 2.0 * a2 + a, a3 - a2, -2.0 * a3 + 3.0 * a2);
            [0, 1, 2].map(|k| h0 * sp[k] + h1 * st[k] + h2 * et[k] + h3 * ep[k])
        };
        let tan = |a: f64| -> [f64; 3] {
            [0, 1, 2].map(|k| {
                let c = 6.0 * sp[k] + 3.0 * st[k] + 3.0 * et[k] - 6.0 * ep[k];
                let d = -6.0 * sp[k] - 4.0 * st[k] - 2.0 * et[k] + 6.0 * ep[k];
                c * a * a + d * a + st[k]
            })
        };
        let norm = |v: [f64; 3]| -> [f64; 3] {
            let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
            if l > 1e-8 { v.map(|x| x / l) } else { [0.0; 3] }
        };
        let cross = |a: [f64; 3], b: [f64; 3]| [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
        let (mut spos, sdir) = if alpha < min_t {
            let t = tan(min_t);
            let p0 = pos(min_t);
            ([0, 1, 2].map(|k| p0[k] + t[k] * (alpha - min_t)), norm(t))
        } else if alpha > max_t {
            let t = tan(max_t);
            let p0 = pos(max_t);
            ([0, 1, 2].map(|k| p0[k] + t[k] * (alpha - max_t)), norm(t))
        } else {
            (pos(alpha), norm(tan(alpha)))
        };
        let h = if self.smooth_interp_roll_scale {
            let x = alpha.clamp(0.0, 1.0);
            if alpha >= 1.0 { 1.0 } else { x * x * (3.0 - 2.0 * x) }
        } else {
            alpha
        };
        let bx = norm(cross(self.spline_up_dir, sdir));
        let by = norm(cross(sdir, bx));
        let off = [0, 1].map(|k| self.start_offset[k] + (self.end_offset[k] - self.start_offset[k]) * h);
        for k in 0..3 {
            spos[k] += off[0] * bx[k] + off[1] * by[k];
        }
        let roll = self.start_roll + (self.end_roll - self.start_roll) * h;
        let (c, s) = (roll.cos(), roll.sin());
        let xv = [0, 1, 2].map(|k| c * bx[k] - s * by[k]);
        let yv = [0, 1, 2].map(|k| c * by[k] + s * bx[k]);
        let sc = [0, 1].map(|k| self.start_scale[k] + (self.end_scale[k] - self.start_scale[k]) * h);
        // FTransform(X axis, Y axis, Z axis, origin) with Scale3D per ForwardAxis
        let (axes, scale) = match self.forward_axis {
            1 => ([yv, sdir, xv], [sc[1], 1.0, sc[0]]),
            2 => ([xv, yv, sdir], [sc[0], sc[1], 1.0]),
            _ => ([sdir, xv, yv], [1.0, sc[0], sc[1]]),
        };
        let mut q = p;
        q[ax] = 0.0;
        [0, 1, 2].map(|k| spos[k] + axes[0][k] * q[0] * scale[0] + axes[1][k] * q[1] * scale[1] + axes[2][k] * q[2] * scale[2])
    }
}

/// The Ultra_Dynamic_Sky actor's material values (scripts/shaders/uds_values.py, read straight from the level): its
/// Ultra_Dynamic_Sky_Sphere component's OverrideMaterials[0] (a MaterialInstanceDynamic the Blueprint construction
/// script saved in the level) over its Parent MIC; preshader.py instance_values merge (later wins)
#[derive(Clone, Debug, Default)]
pub struct Uds {
    pub level: usize,
    pub actor: String,
    /// actor root world transform
    pub xf: Xf,
    /// the root component's RelativeLocation (uds_values.py actor_location)
    pub actor_location: [f64; 3],
    /// actor properties over the Blueprint class defaults
    pub props: Map<String, Value>,
    pub mid: String,
    pub mic: String,
    pub scalars: BTreeMap<String, f64>,
    pub vectors: BTreeMap<String, [f64; 4]>,
}

/// A DecalComponent (ADecalActor's or a Blueprint's): projected material in a box. UDecalComponent: DecalMaterial,
/// DecalSize (half extents in component space; default (128, 256, 256), CUE4Parse ADecalActor.cs:18), SortOrder,
/// FadeScreenSize, FadeStartDelay, FadeDuration (tagged properties). The box is `xf * scale(DecalSize)`, projecting
/// along the component's -X (UE decal convention) [UNCONFIRMED: projection axis from UE source recalled]
#[derive(Clone, Debug)]
pub struct Decal {
    pub level: usize,
    pub name: String,
    pub actor: String,
    pub xf: Xf,
    /// material object path "pkg.N"
    pub material: String,
    pub size: [f64; 3],
    pub sort_order: i64,
    pub fade_screen_size: f64,
    pub props: Map<String, Value>,
}

/// A reflection capture component (Sphere/Box/Plane ReflectionCaptureComponent) and its cooked cubemap
#[derive(Clone, Debug)]
pub struct ReflectionCaptureRec {
    pub level: usize,
    pub kind: String,
    pub name: String,
    pub actor: String,
    pub xf: Xf,
    /// InfluenceRadius (sphere, default 3000), BoxTransitionDistance (box), Brightness, CaptureOffset, ...
    pub props: Map<String, Value>,
    pub map_build_data_id: Option<String>,
}

/// HLOD proxy (LODActor StaticMeshComponent0): drawn instead of `subs` when the view is farther than `min_draw` from
/// the proxy box (FLODSceneTree::UpdateVisibilityStates, .text 0x22c1d30; ue_level.gd hlod_record header)
#[derive(Clone, Debug)]
pub struct Hlod {
    pub level: usize,
    pub name: String,
    pub lod_level: i64,
    pub mesh: String,
    pub mesh_name: String,
    pub xf: Xf,
    pub min_draw: f64,
    pub box_origin: [f64; 3],
    pub box_extent: [f64; 3],
    pub subs: Vec<String>,
}

/// PlayerStart (native or Blueprint subclass); team from the instance, else the Blueprint class defaults
#[derive(Clone, Debug)]
pub struct Spawn {
    pub level: usize,
    pub name: String,
    pub class: String,
    pub xf: Xf,
    pub team: Option<Value>,
}

/// A light or atmosphere component with its merged properties (values absent are class defaults, see ue_light.gd DEF)
#[derive(Clone, Debug)]
pub struct LightRec {
    pub level: usize,
    pub kind: String,
    pub name: String,
    pub xf: Xf,
    pub props: Map<String, Value>,
    /// Mobility Static in a level with build data: lighting only in the lightmaps (no run-time light)
    pub baked: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Lighting {
    pub sun: Option<LightRec>,
    pub locals: Vec<LightRec>,
    pub sky_light: Option<LightRec>,
    pub fog: Option<LightRec>,
    pub atmos: Option<LightRec>,
    /// unbound PostProcessVolume Settings
    pub post: Option<LightRec>,
    /// BP_Sky_Sphere_C instance (props) - its class defaults are `Pkgs::defaults("Engine/Content/EngineSky/BP_Sky_Sphere")`
    pub sky: Option<LightRec>,
    /// SM_SkySphere radius x the component's RelativeScale3D.X (bounds the fog's view ray for sky pixels)
    pub sky_radius_cm: f64,
    /// Ultra_Dynamic_Sky actor: (level index, actor world transform); its own mesh components are skipped as "sky"
    pub uds: Option<(usize, Xf)>,
}

/// A brush volume (NavMeshBoundsVolume, ...): convex hull points in world space and their box
#[derive(Clone, Debug)]
pub struct Volume {
    pub level: usize,
    pub class: String,
    pub name: String,
    pub xf: Xf,
    /// all hull points (world)
    pub points: Vec<[f64; 3]>,
    /// the brush's convex elements separately (world): what collision uses
    pub elems: Vec<Vec<[f64; 3]>>,
    /// the volume actor ("pkg.N") and its BrushComponent ("pkg.N")
    pub actor_path: String,
    pub component_path: String,
    pub min: [f64; 3],
    pub max: [f64; 3],
}

/// A spectator (BP_CrowdSystemActor_*): skeletal mesh + idle animation 0 (UeLevel.crowd)
#[derive(Clone, Debug)]
pub struct Crowd {
    pub level: usize,
    pub name: String,
    pub class: String,
    pub mesh: String,
    pub anim: String,
    pub materials: Vec<String>,
    pub xf: Xf,
}

#[derive(Clone, Debug, Default)]
pub struct LevelData {
    pub map: String,
    pub levels: Vec<LevelRef>,
    pub actors: Vec<Actor>,
    pub meshes: Vec<MeshPlacement>,
    pub hlods: Vec<Hlod>,
    pub spawns: Vec<Spawn>,
    pub lights: Lighting,
    pub volumes: Vec<Volume>,
    pub crowd: Vec<Crowd>,
    /// the first DirectionalLight actor's root transform (UeLevel.read "sun")
    pub sun_actor: Option<Xf>,
    /// mesh components seen
    pub seen: usize,
    /// why -> component names not placed (hlod, hidden, sky, no_mesh, empty, spline); spline components are in
    /// `splines` (the Godot reader skips them; the name stays in this list for parity)
    pub skips: BTreeMap<String, Vec<String>>,
    pub splines: Vec<SplineMeshPlacement>,
    pub landscape: Vec<crate::landscape::LandscapeComponent>,
    /// per level (index as `levels`): its MapBuildDataRegistry, None when unbuilt or not decodable
    pub build: Vec<Option<crate::builddata::BuildData>>,
    pub uds: Option<Uds>,
    pub landscape_actors: Vec<crate::landscape::LandscapeActor>,
    pub landscape_collision: Vec<crate::landscape::LandscapeCollision>,
    /// LandscapeGrassType object path -> its tagged properties (GrassVarieties: GrassMesh, GrassDensity, ...)
    pub grass_types: BTreeMap<String, Map<String, Value>>,
    pub decals: Vec<Decal>,
    pub reflection_captures: Vec<ReflectionCaptureRec>,
    /// UModel exports per level (brush models of volumes; no ModelComponent is cooked, so no BSP render geometry)
    pub bsp_models: usize,
    /// every placed Blueprint actor (actors.rs); filled by `read`
    pub gameplay: Vec<crate::actors::GameplayActor>,
    /// their classes by export Type (chain, native root, category, class defaults)
    pub gameplay_classes: BTreeMap<String, crate::actors::GameplayClass>,
}

impl LevelData {
    pub fn statics(&self) -> impl Iterator<Item = &MeshPlacement> {
        self.meshes.iter().filter(|m| !m.is_instanced())
    }
    pub fn instanced(&self) -> impl Iterator<Item = &MeshPlacement> {
        self.meshes.iter().filter(|m| m.is_instanced())
    }
    /// Baked lighting of a placement (LOD 0's MapBuildDataId in its level's registry)
    pub fn build_data(&self, level: usize, lods: &[crate::native::LodInfo]) -> Option<&crate::builddata::MeshBuildData> {
        let id = lods.first()?.map_build_data_id.as_ref()?;
        self.build.get(level)?.as_ref()?.meshes.get(id)
    }
    /// Cooked cubemap of a reflection capture (its MapBuildDataId in its level's registry)
    pub fn reflection_data(&self, c: &ReflectionCaptureRec) -> Option<&crate::vlm::ReflectionCapture> {
        self.build.get(c.level)?.as_ref()?.reflections.get(c.map_build_data_id.as_ref()?)
    }
    /// The volumetric lightmap drawn for the map: the first level (persistent first) whose registry has one
    pub fn volumetric_lightmap(&self) -> Option<&crate::vlm::VolumetricLightmap> {
        self.build.iter().flatten().flat_map(|b| b.volumetric.iter()).map(|(_, v)| v).next()
    }
    pub fn skip_counts(&self) -> BTreeMap<String, usize> {
        self.skips.iter().map(|(k, v)| (k.clone(), v.len())).collect()
    }
}

// ---------------------------------------------------------------- read

/// Persistent map + every streaming sub-level, depth first (UeLevel.levels)
pub fn levels(pk: &Pkgs, map_pkg: &str) -> Vec<LevelRef> {
    let mut out = Vec::new();
    levels_into(pk, map_pkg, IDENTITY, &mut out, 0);
    out
}

fn levels_into(pk: &Pkgs, map_pkg: &str, xf: Xf, out: &mut Vec<LevelRef>, depth: usize) {
    out.push(LevelRef { pkg: map_pkg.to_string(), xf });
    if depth > 16 {
        return;
    }
    for e in pk.load_pkg(map_pkg).iter() {
        if !ty(e).starts_with("LevelStreaming") {
            continue;
        }
        let p = props_of(e);
        let wa = p.get("WorldAsset").map(|w| s(w, "AssetPathName")).unwrap_or("");
        if wa.is_empty() {
            continue;
        }
        let lx = xf * p.get("LevelTransform").map(ftransform).unwrap_or(IDENTITY);
        levels_into(pk, &game_pkg(wa), lx, out, depth + 1);
    }
}

fn is_mesh_comp(e: &Value) -> bool {
    let t = ty(e);
    t.ends_with("StaticMeshComponent") || t == "SplineMeshComponent" || e.get("PerInstanceSMData").is_some()
}

fn is_start(t: &str) -> bool {
    t == "MordhauPlayerStart" || t == "PlayerStart" || (t.ends_with("PlayerStart_C") && t.starts_with("BP_"))
}

fn mat_paths(p: &Map<String, Value>) -> Vec<String> {
    p.get("OverrideMaterials")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().map(|m| obj_path(Some(m)).to_string()).collect())
        .unwrap_or_default()
}

fn v2(v: Option<&Value>, d: f64) -> [f64; 2] {
    let g = |k: &str| v.and_then(|v| v.get(k)).and_then(|x| x.as_f64()).unwrap_or(d);
    [g("X"), g("Y")]
}

fn mats(p: &Map<String, Value>) -> Vec<String> {
    p.get("OverrideMaterials")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().map(|m| if m.is_object() { pkg_of(obj_path(Some(m))).to_string() } else { String::new() }).collect())
        .unwrap_or_default()
}

const SKY_SPHERE: &str = "Engine/Content/EngineSky/SM_SkySphere";
const UDS_MESHES: &str = "Mordhau/Content/UltraDynamicSky/Meshes/";

/// Read a map (e.g. "Mordhau/Content/Mordhau/Maps/Arena_Map/DU_Arena") and every streamed level
pub fn read(pk: &Pkgs, map_pkg: &str) -> LevelData {
    let mut d = LevelData { map: map_pkg.to_string(), levels: levels(pk, map_pkg), ..Default::default() };
    let lvs = d.levels.clone();
    read_levels(pk, &mut d, &lvs);
    crate::actors::read(pk, &mut d);
    // grass type assets referenced by the landscape's baked grass data
    let mut gts: Vec<String> = d.landscape.iter().flat_map(|c| c.grass.iter().flat_map(|g| g.weights.iter().map(|w| w.0.clone()))).collect();
    gts.sort();
    gts.dedup();
    for g in gts {
        if let Some(o) = pk.obj(Some(&serde_json::json!({ "ObjectPath": g }))) {
            d.grass_types.insert(g, pk.props(&o));
        }
    }
    d
}

fn read_levels(pk: &Pkgs, d: &mut LevelData, lvs: &[LevelRef]) {
    for (li, lv) in lvs.iter().enumerate() {
        let exps = pk.load_pkg(&lv.pkg);
        let lx = lv.xf;
        // the level object and whether it has build data (UeLightmap.registry non-empty)
        let level_idx = exps.iter().position(|e| ty(e) == "Level");
        let built = level_idx.and_then(|i| pk.obj(props_of(&exps[i]).get("MapBuildData"))).is_some();
        d.build.push(if built { crate::builddata::read(pk, &lv.pkg) } else { None });
        let mut tex_cache = HashMap::new();
        for (ei, e) in exps.iter().enumerate() {
            let t = ty(e);
            let ep = props_of(e);
            // actors: exports whose Outer is the level object
            if let (Some(l), Some(op)) = (level_idx, e.get("Outer").map(|o| obj_path(Some(o)))) {
                if op.rsplit_once('.').and_then(|(_, i)| i.parse::<usize>().ok()) == Some(l) && ei != l {
                    let rc = pk.obj(ep.get("RootComponent"));
                    d.actors.push(Actor {
                        level: li,
                        name: s(e, "Name").to_string(),
                        class: t.to_string(),
                        xf: rc.map(|rc| lx * pk.world_xf(&rc)),
                        hidden: ep.get("bHidden").and_then(|v| v.as_bool()).unwrap_or(false),
                    });
                }
            }
            if is_start(t) {
                // Blueprint subclasses too (FFA_Arena: 14 BP_MordhauPlayerStart_C). Team: instance value, else the
                // Blueprint's class defaults (UePkg.defaults), else absent.
                let rc = pk.obj(ep.get("RootComponent"));
                let mut team = ep.get("Team").cloned();
                if team.is_none() && t.ends_with("_C") {
                    team = pk.defaults(&class_pkg(e)).get("Team").cloned();
                }
                d.spawns.push(Spawn {
                    level: li,
                    name: s(e, "Name").to_string(),
                    class: t.to_string(),
                    xf: lx * rc.map(|rc| pk.world_xf(&rc)).unwrap_or(IDENTITY),
                    team,
                });
            } else if t == "DirectionalLight" && d.sun_actor.is_none() {
                d.sun_actor = Some(lx * pk.obj(ep.get("RootComponent")).map(|rc| pk.world_xf(&rc)).unwrap_or(IDENTITY));
            } else if t.contains("Ultra_Dynamic_Sky") && d.lights.uds.is_none() {
                let rc = pk.obj(ep.get("RootComponent"));
                d.lights.uds = Some((li, lx * rc.map(|rc| pk.world_xf(&rc)).unwrap_or(IDENTITY)));
            } else if t.ends_with("Volume") {
                if let Some(v) = volume(pk, li, lx, e, &lv.pkg, ei) {
                    d.volumes.push(v);
                }
            }
            light(pk, &mut d.lights, li, lx, built, e);
            if t == "LandscapeComponent" {
                if let Some(c) = crate::landscape::component(pk, li, lx, &Obj(exps.clone(), ei), &mut tex_cache) {
                    d.landscape.push(c);
                }
            }
            if t == "Model" {
                d.bsp_models += 1;
            }
            if t == "LandscapeHeightfieldCollisionComponent" {
                if let Some(c) = crate::landscape::collision(pk, li, lx, &Obj(exps.clone(), ei)) {
                    d.landscape_collision.push(c);
                }
            }
            if matches!(t, "Landscape" | "LandscapeProxy" | "LandscapeStreamingProxy") {
                d.landscape_actors.push(crate::landscape::actor(pk, li, lx, e));
            }
            if t == "DecalComponent" {
                let p = pk.props(e);
                let a = pk.obj(e.get("Outer"));
                d.decals.push(Decal {
                    level: li,
                    name: format!("{}.{}", a.as_ref().map(|a| s(a, "Name")).unwrap_or("?"), s(e, "Name")),
                    actor: a.as_ref().map(|a| s(a, "Name")).unwrap_or("?").to_string(),
                    xf: lx * pk.world_xf(e),
                    material: obj_path(p.get("DecalMaterial")).to_string(),
                    size: p.get("DecalSize").map(|v| vec3(Some(v), 0.0)).unwrap_or([128.0, 256.0, 256.0]),
                    sort_order: p.get("SortOrder").and_then(|v| v.as_i64()).unwrap_or(0),
                    fade_screen_size: p.get("FadeScreenSize").and_then(|v| v.as_f64()).unwrap_or(0.01),
                    props: p,
                });
            }
            if t.ends_with("ReflectionCaptureComponent") {
                let p = pk.props(e);
                let a = pk.obj(e.get("Outer"));
                d.reflection_captures.push(ReflectionCaptureRec {
                    level: li,
                    kind: t.to_string(),
                    name: format!("{}.{}", a.as_ref().map(|a| s(a, "Name")).unwrap_or("?"), s(e, "Name")),
                    actor: a.as_ref().map(|a| s(a, "Name")).unwrap_or("?").to_string(),
                    xf: lx * pk.world_xf(e),
                    map_build_data_id: p.get("MapBuildDataId").and_then(|v| v.as_str()).map(|g| g.replace('-', "")),
                    props: p,
                });
            }
            if t.contains("Ultra_Dynamic_Sky") && d.uds.is_none() {
                d.uds = uds(pk, li, lx, &exps, e);
            }
            if t == "SkeletalMeshComponent" {
                if let Some(c) = crowd(pk, li, lx, e) {
                    d.crowd.push(c);
                }
            }
            if !is_mesh_comp(e) {
                continue;
            }
            d.seen += 1;
            let actor = pk.obj(e.get("Outer"));
            let aname = actor.as_ref().map(|a| s(a, "Name")).unwrap_or("?").to_string();
            let name = format!("{aname}.{}", s(e, "Name"));
            let p = pk.props(e);
            let mesh = obj_path(p.get("StaticMesh").filter(|v| v.is_object())).to_string();
            if actor.as_ref().is_some_and(|a| ty(a) == "LODActor") {
                // HLOD proxy: not a placed mesh (still counted as an "hlod" skip) but recorded for the HLOD switch
                add_skip(&mut d.skips, "hlod", name);
                if let Some(h) = hlod(pk, li, actor.as_ref().unwrap(), &p, &mesh, lx * pk.world_xf(e)) {
                    d.hlods.push(h);
                }
                continue;
            }
            if actor.as_ref().is_some_and(|a| props_of(a).get("bHidden").and_then(|v| v.as_bool()).unwrap_or(false)) {
                add_skip(&mut d.skips, "hidden", name);
                continue;
            }
            if t == "SplineMeshComponent" {
                add_skip(&mut d.skips, "spline", name.clone());
                if !mesh.is_empty() {
                    let sp = p.get("SplineParams");
                    let g = |k: &str| sp.and_then(|s| s.get(k));
                    let gf = |k: &str| g(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    let def3 = |k: &str, d: [f64; 3]| g(k).map(|v| vec3(Some(v), 0.0)).unwrap_or(d);
                    let axis = p.get("ForwardAxis").and_then(|v| v.as_str()).unwrap_or("ESplineMeshAxis::X");
                    d.splines.push(SplineMeshPlacement {
                        level: li,
                        name,
                        actor: aname,
                        xf: lx * pk.world_xf(e),
                        materials: mats(&p),
                        material_paths: mat_paths(&p),
                        cast_shadow: p.get("CastShadow").and_then(|v| v.as_bool()).unwrap_or(true),
                        start_pos: def3("StartPos", [0.0; 3]),
                        start_tangent: def3("StartTangent", [100.0, 0.0, 0.0]),
                        start_scale: v2(g("StartScale"), 1.0),
                        start_roll: gf("StartRoll"),
                        start_offset: v2(g("StartOffset"), 0.0),
                        end_pos: def3("EndPos", [100.0, 0.0, 0.0]),
                        end_tangent: def3("EndTangent", [100.0, 0.0, 0.0]),
                        end_scale: v2(g("EndScale"), 1.0),
                        end_roll: gf("EndRoll"),
                        end_offset: v2(g("EndOffset"), 0.0),
                        forward_axis: if axis.ends_with("::Y") { 1 } else if axis.ends_with("::Z") { 2 } else { 0 },
                        spline_up_dir: p.get("SplineUpDir").map(|v| vec3(Some(v), 0.0)).unwrap_or([0.0, 0.0, 1.0]),
                        spline_boundary_min: p.get("SplineBoundaryMin").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        spline_boundary_max: p.get("SplineBoundaryMax").and_then(|v| v.as_f64()).unwrap_or(0.0),
                        smooth_interp_roll_scale: p.get("bSmoothInterpRollScale").and_then(|v| v.as_bool()).unwrap_or(false),
                        lods: crate::native::mesh_component(pk, &lv.pkg, ei, false).map(|n| n.lods).unwrap_or_default(),
                        component_path: format!("{}.{ei}", lv.pkg),
                        mesh,
                    });
                }
                continue;
            }
            if mesh.is_empty() {
                add_skip(&mut d.skips, "no_mesh", name);
                continue;
            }
            // BP_Sky_Sphere's material is a run-time MaterialInstanceDynamic; Ultra_Dynamic_Sky_BP's own components are
            // drawn by the sky renderer (ue_uds.gd), never as their grid-material meshes
            if pkg_of(&mesh) == SKY_SPHERE || pkg_of(&mesh).starts_with(UDS_MESHES) {
                add_skip(&mut d.skips, "sky", name);
                continue;
            }
            let mut rec = MeshPlacement {
                level: li,
                name: name.clone(),
                actor: aname,
                component: t.to_string(),
                mesh,
                xf: lx * pk.world_xf(e),
                materials: mats(&p),
                material_paths: mat_paths(&p),
                // UPrimitiveComponent CastShadow: absent = true [UE source recalled, UNCONFIRMED; ue_level.gd]
                cast_shadow: p.get("CastShadow").and_then(|v| v.as_bool()).unwrap_or(true),
                instances: vec![],
                lods: vec![],
                component_path: format!("{}.{ei}", lv.pkg),
                hidden_in_game: hidden_in_game(pk, e, actor.as_deref()),
            };
            // extract/json carries PerInstanceSMData / LODData (CUE4Parse decodes them natively); mh-pak's export JSON
            // holds tagged properties only, so they are read from the export bytes (native::mesh_component)
            let instanced = e.get("PerInstanceSMData").is_some() || t.contains("Instanced");
            let nat = crate::native::mesh_component(pk, &lv.pkg, ei, instanced);
            if let Some(n) = &nat {
                rec.lods = n.lods.clone();
            }
            if instanced {
                rec.instances = match e.get("PerInstanceSMData").and_then(|v| v.as_array()) {
                    Some(a) => a.iter().map(|d| ftransform(d.get("TransformData").unwrap_or(&Value::Null))).collect(),
                    None => nat.and_then(|n| n.instances).unwrap_or_default(),
                };
                if rec.instances.is_empty() {
                    add_skip(&mut d.skips, "empty", name);
                    continue;
                }
            }
            d.meshes.push(rec);
        }
    }
}

fn add_skip(m: &mut BTreeMap<String, Vec<String>>, why: &str, name: String) {
    m.entry(why.to_string()).or_default().push(name);
}

/// HLOD proxy record (UeLevel.hlod_record)
fn hlod(pk: &Pkgs, li: usize, actor: &Value, p: &Map<String, Value>, mesh: &str, xf: Xf) -> Option<Hlod> {
    if mesh.is_empty() {
        return None;
    }
    let ap = props_of(actor);
    let mobj = pk.obj(p.get("StaticMesh"));
    let b = mobj.as_ref().and_then(|m| props_of(m).get("ExtendedBounds").cloned()).unwrap_or(Value::Null);
    let subs = ap
        .get("SubActors")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter(|x| x.is_object())
                .map(|x| {
                    // "Class'Level.Persistent:Actor'" -> slice between quotes, then the part after the first "."
                    let on = s(x, "ObjectName");
                    let q = on.split('\'').nth(1).unwrap_or("");
                    q.split('.').nth(1).unwrap_or("").to_string()
                })
                .collect()
        })
        .unwrap_or_default();
    let ap_f = |k: &str| ap.get(k).and_then(|v| v.as_f64());
    Some(Hlod {
        level: li,
        name: s(actor, "Name").to_string(),
        lod_level: ap.get("LODLevel").and_then(|v| v.as_f64()).map_or(1, |x| x as i64),
        mesh: mesh.to_string(),
        mesh_name: mobj.as_ref().map(|m| s(m, "Name").to_string()).unwrap_or_default(),
        xf,
        min_draw: p.get("MinDrawDistance").and_then(|v| v.as_f64()).or_else(|| ap_f("LODDrawDistance")).unwrap_or(0.0),
        box_origin: vec3(b.get("Origin"), 0.0),
        box_extent: vec3(b.get("BoxExtent"), 0.0),
        subs,
    })
}

/// Lighting components (UeLight.read)
fn light(pk: &Pkgs, l: &mut Lighting, li: usize, lx: Xf, built: bool, e: &Value) {
    let t = ty(e);
    let name = || {
        let a = pk.obj(e.get("Outer"));
        format!("{}.{}", a.as_ref().map(|a| s(a, "Name")).unwrap_or("?"), s(e, "Name"))
    };
    let rec = |props: Map<String, Value>, xf: Xf| LightRec { level: li, kind: t.to_string(), name: name(), xf, props, baked: false };
    match t {
        "DirectionalLightComponent" | "PointLightComponent" | "RectLightComponent" | "SpotLightComponent" => {
            let mut r = rec(pk.props(e), lx * pk.world_xf(e));
            // Mobility Static = lighting only in the level's lightmaps. The class default is Stationary
            // (ue_light.gd: the sun serializes no Mobility yet owns shadowmap channel 0)
            r.baked = built && r.props.get("Mobility").and_then(|v| v.as_str()) == Some("EComponentMobility::Static");
            if t == "DirectionalLightComponent" {
                if l.sun.is_none() {
                    l.sun = Some(r);
                } else {
                    // (rust-armory) further directional lights light the scene too (UE draws every directional
                    // light; MainMenu's FillLight / HeadLight / RimLight): kept with the local lights
                    l.locals.push(r);
                }
            } else {
                l.locals.push(r);
            }
        }
        "SkyLightComponent" => l.sky_light = Some(rec(pk.props(e), lx * pk.world_xf(e))),
        "ExponentialHeightFogComponent" => l.fog = Some(rec(pk.props(e), lx * pk.world_xf(e))),
        "AtmosphericFogComponent" => l.atmos = Some(rec(pk.props(e), lx * pk.world_xf(e))),
        "PostProcessVolume" => {
            let pp = props_of(e);
            if pp.get("bUnbound").and_then(|v| v.as_bool()).unwrap_or(false) {
                let st = pp.get("Settings").and_then(|v| v.as_object()).cloned().unwrap_or_default();
                l.post = Some(rec(st, IDENTITY));
            }
        }
        "StaticMeshComponent" => {
            let mp = pk.props(e);
            if pkg_of(obj_path(mp.get("StaticMesh").filter(|v| v.is_object()))) == SKY_SPHERE {
                let mut r = 0.0;
                for m in pk.load_pkg(SKY_SPHERE).iter() {
                    if ty(m) == "StaticMesh" {
                        r = props_of(m).get("ExtendedBounds").and_then(|b| b.get("SphereRadius")).and_then(|v| v.as_f64()).unwrap_or(0.0);
                    }
                }
                let sx = mp.get("RelativeScale3D").and_then(|v| v.get("X")).and_then(|v| v.as_f64()).unwrap_or(1.0);
                l.sky_radius_cm = r * sx;
            }
        }
        "BP_Sky_Sphere_C" => l.sky = Some(rec(pk.props(e), IDENTITY)),
        _ => {}
    }
}

/// A brush volume's convex collision (BrushComponent BrushBodySetup AggGeom ConvexElems VertexData) through the
/// volume's root world transform (ModeData.nav_bounds)
fn volume(pk: &Pkgs, li: usize, lx: Xf, e: &Value, pkg: &str, ei: usize) -> Option<Volume> {
    let p = props_of(e);
    let root = pk.obj(p.get("RootComponent"))?;
    // the RootComponent is the volume's BrushComponent
    let body = pk.obj(props_of(&root).get("BrushBodySetup"))?;
    let mut pts_local = vec![];
    let mut elems_local: Vec<Vec<[f64; 3]>> = vec![];
    for ce in props_of(&body).get("AggGeom")?.get("ConvexElems")?.as_array()? {
        elems_local.push(ce.get("VertexData").and_then(|v| v.as_array()).into_iter().flatten().map(|v| vec3(Some(v), 0.0)).collect());
        for v in ce.get("VertexData").and_then(|v| v.as_array()).into_iter().flatten() {
            pts_local.push(vec3(Some(v), 0.0));
        }
    }
    if pts_local.is_empty() {
        return None;
    }
    let xf = lx * pk.world_xf(&root);
    let points: Vec<[f64; 3]> = pts_local.iter().map(|q| xf.apply(*q)).collect();
    let mut min = points[0];
    let mut max = points[0];
    for q in &points {
        for k in 0..3 {
            min[k] = min[k].min(q[k]);
            max[k] = max[k].max(q[k]);
        }
    }
    let elems = elems_local.iter().map(|el| el.iter().map(|q| xf.apply(*q)).collect()).collect();
    Some(Volume {
        level: li,
        class: ty(e).to_string(),
        name: s(e, "Name").to_string(),
        xf,
        points,
        elems,
        actor_path: format!("{}.{ei}", pkg),
        component_path: obj_path(p.get("RootComponent")).to_string(),
        min,
        max,
    })
}

/// Spectators (UeLevel.crowd): the SkeletalMeshComponent's properties from the class chain's _GEN_VARIABLE templates,
/// then the instance; the idle animation = the class defaults' IdleAnimations[0]
fn crowd(pk: &Pkgs, li: usize, lx: Xf, e: &Value) -> Option<Crowd> {
    let actor = pk.obj(e.get("Outer"))?;
    let cls = class_pkg(&actor);
    if !cls.rsplit('/').next().unwrap_or("").starts_with("BP_CrowdSystemActor") {
        return None;
    }
    let comp = s(e, "Name");
    let mut pr = Map::new();
    for c in class_chain(pk, &cls) {
        for g in pk.load_pkg(&c).iter() {
            if s(g, "Name") == format!("{comp}_GEN_VARIABLE") {
                for (k, v) in props_of(g) {
                    pr.insert(k.clone(), v.clone());
                }
                break;
            }
        }
    }
    for (k, v) in props_of(e) {
        pr.insert(k.clone(), v.clone());
    }
    let defs = pk.defaults(&cls);
    let anim = defs.get("IdleAnimations").and_then(|v| v.as_array()).and_then(|a| a.first()).filter(|v| v.is_object());
    let par = pk.obj(pr.get("AttachParent"));
    let x = rel_xf(&pr);
    let xf = lx * match par {
        Some(par) => pk.world_xf(&par) * x,
        None => x,
    };
    Some(Crowd {
        level: li,
        name: format!("{}.{comp}", s(&actor, "Name")),
        class: cls,
        mesh: pkg_of(obj_path(pr.get("SkeletalMesh"))).to_string(),
        anim: anim.map(|a| pkg_of(obj_path(Some(a))).to_string()).unwrap_or_default(),
        materials: mats(&pr),
        xf,
    })
}

/// Super chain of a Blueprint class package, root first (UeLevel.class_chain)
pub fn class_chain(pk: &Pkgs, pkg: &str) -> Vec<String> {
    let mut out = vec![];
    let mut p = pkg.to_string();
    while !p.is_empty() && p.starts_with("Mordhau/Content") && out.len() < 64 {
        let a = pk.load_pkg(&p);
        if a.is_empty() {
            break;
        }
        out.insert(0, p.clone());
        p = mpkg::export_of(&a, "BlueprintGeneratedClass").map(|b| pkg_of(obj_path(b.get("Super"))).to_string()).unwrap_or_default();
    }
    out
}

/// Ultra_Dynamic_Sky values (scripts/shaders/uds_values.py): the actor, its Ultra_Dynamic_Sky_Sphere component's
/// OverrideMaterials[0] (MID) and that MID's Parent (MIC); ScalarParameterValues / VectorParameterValues of MIC then MID
fn uds(pk: &Pkgs, li: usize, lx: Xf, exps: &Rc<Vec<Value>>, actor: &Value) -> Option<Uds> {
    let an = s(actor, "Name").to_string();
    let sphere = exps.iter().find(|e| {
        ty(e) == "StaticMeshComponent"
            && s(e, "Name") == "Ultra_Dynamic_Sky_Sphere"
            && pk.obj(e.get("Outer")).is_some_and(|o| s(&o, "Name") == an)
    })?;
    let sp = pk.props(sphere);
    let mid_ref = sp.get("OverrideMaterials").and_then(|v| v.as_array()).and_then(|a| a.first())?;
    let mid = pk.obj(Some(mid_ref))?;
    let mic_path = obj_path(props_of(&mid).get("Parent")).to_string();
    let mic = pk.obj(props_of(&mid).get("Parent"));
    let mut u = Uds { level: li, actor: an, mid: obj_path(Some(mid_ref)).to_string(), mic: pkg_of(&mic_path).to_string(), ..Default::default() };
    let mut plist: Vec<&Map<String, Value>> = mic.iter().map(|m| props_of(m)).collect();
    plist.push(props_of(&mid));
    for p in plist {
        for e in p.get("ScalarParameterValues").and_then(|v| v.as_array()).into_iter().flatten() {
            let n = e.get("ParameterInfo").map(|i| s(i, "Name")).unwrap_or("").to_string();
            u.scalars.insert(n, e.get("ParameterValue").and_then(|v| v.as_f64()).unwrap_or(0.0));
        }
        for e in p.get("VectorParameterValues").and_then(|v| v.as_array()).into_iter().flatten() {
            let n = e.get("ParameterInfo").map(|i| s(i, "Name")).unwrap_or("").to_string();
            let c = e.get("ParameterValue");
            let g = |k: &str| c.and_then(|c| c.get(k)).and_then(|v| v.as_f64()).unwrap_or(0.0);
            u.vectors.insert(n, [g("R"), g("G"), g("B"), g("A")]);
        }
    }
    let ap = props_of(actor);
    if let Some(rc) = pk.obj(ap.get("RootComponent")) {
        u.actor_location = vec3(props_of(&rc).get("RelativeLocation"), 0.0);
        u.xf = lx * pk.world_xf(&rc);
    }
    let mut props = (*pk.defaults(&class_pkg(actor))).clone();
    for (k, v) in ap {
        props.insert(k.clone(), v.clone());
    }
    u.props = props;
    Some(u)
}

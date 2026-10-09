//! Exports of a package turned into the JSON shape `mdx json` (CUE4Parse) writes to extract/json, so readers can use
//! either source and equivalence is checkable value by value. Port of the export half of `ue_asset.gd` `UeAsset`
//! (object references, tagged properties, SuperStruct, DataTable rows, UEnum names, USkeleton reference skeleton).
//!
//! UE 4.26 names: FPropertyTag operator<< (PropertyTag.cpp), UStruct::SerializeTaggedProperties (Class.cpp),
//! UObject::Serialize (Obj.cpp), UDataTable::LoadStructData (Engine/Private/DataTable.cpp). CUE4Parse references are
//! `file:line` under tools/CUE4Parse-src/CUE4Parse/.
//!
//! A `Reader` owns a package cache (import resolution opens the packages an import points into). It is single-threaded
//! (`Rc`); give each thread its own `Reader` over a shared `Arc<Vfs>`.

use crate::asset::{AssetError, Package, RF_CLASS_DEFAULT_OBJECT};
use crate::buf::Cursor;
use crate::vfs::Vfs;
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// packages kept open (headers only: export bytes are views into the pak mappings)
pub const CACHE_MAX: usize = 512;

/// UStruct exports: after UObject's tagged properties (+ Guid) comes SuperStruct (UStruct.cs:18-37)
pub const STRUCT_TYPES: &[&str] = &[
    "BlueprintGeneratedClass", "WidgetBlueprintGeneratedClass", "AnimBlueprintGeneratedClass", "Function",
    "DelegateFunction", "SparseDelegateFunction", "UserDefinedStruct", "ScriptStruct", "Class",
];
/// Structs that UE serializes natively as an FSoftObjectPath
pub const SOFT_PATH_STRUCTS: &[&str] = &["SoftObjectPath", "SoftClassPath", "StringAssetReference", "StringClassReference"];
/// Natively serialized structs this reader does not decode (skipped by tag size, listed in `unsupported`): sequencer,
/// curve and material-input internals no game reader uses (CUE4Parse FScriptStruct.cs:78-198 lists their layouts)
pub const BINARY_STRUCTS: &[&str] = &[
    "Plane", "PerPlatformFloat", "PerPlatformInt", "PerPlatformBool", "SimpleCurveKey", "Sphere", "TwoVectors",
    "MaterialAttributesInput", "ExpressionInput", "ColorMaterialInput", "ScalarMaterialInput", "VectorMaterialInput",
    "Vector2MaterialInput", "SmartName", "FontCharacter", "UniqueNetIdRepl", "MovieSceneFloatChannel",
    "MovieSceneEvaluationKey", "SectionEvaluationDataTree", "LevelSequenceObjectReferenceMap", "MovieSceneSegment",
    "MovieSceneTrackIdentifier", "MovieSceneSequenceID", "MovieSceneEvalTemplatePtr", "MovieSceneTrackImplementationPtr",
    "NameCurveKey", "StringCurveKey", "SkeletalMeshSamplingLODBuiltData", "CompressedRichCurve",
    "MovieSceneSegmentIdentifier", "MovieSceneSubSequenceTree", "MovieSceneEventParameters",
    "MovieSceneSequenceInstanceDataPtr", "MovieSceneEvaluationFieldEntityTree", "MovieSceneFloatValue", "ClothLODData",
    "ClothTetherData", "NiagaraVariable", "NiagaraVariableBase", "NiagaraVariableWithOffset", "Transform3f",
    "MovieSceneTrackFieldData", "MovieSceneSubSectionFieldData", "SkeletalMeshSamplingRegionBuiltData",
    "PerPlatformFrameRate", "PerQualityLevelInt", "PerQualityLevelFloat",
];
/// UEnum exports: after the tagged properties come the enum's own Names and CppForm (UEnum.cs)
pub const ENUM_TYPES: &[&str] = &["UserDefinedEnum", "Enum"];
pub const CPP_FORM: &[&str] = &["Regular", "Namespaced", "EnumClass"]; // UEnum::ECppForm (UEnum.cs)
pub const NET_QUANTIZE: &[&str] =
    &["Vector_NetQuantize", "Vector_NetQuantize10", "Vector_NetQuantize100", "Vector_NetQuantizeNormal"];
// ERichCurveInterpMode / ERichCurveTangentMode / ERichCurveTangentWeightMode (Engine/Classes/Curves/RichCurve.h)
pub const RCIM: &[&str] = &["RCIM_Linear", "RCIM_Constant", "RCIM_Cubic", "RCIM_None"];
pub const RCTM: &[&str] = &["RCTM_Auto", "RCTM_User", "RCTM_Break", "RCTM_None"];
pub const RCTWM: &[&str] = &["RCTWM_WeightedNone", "RCTWM_WeightedArrive", "RCTWM_WeightedLeave", "RCTWM_WeightedBoth"];
const OBJECT_PROPS: &[&str] = &["ObjectProperty", "ClassProperty", "WeakObjectProperty", "InterfaceProperty"];

/// Key of the marker object a value the reader could not decode is replaced with
pub const UNSUPPORTED: &str = "__unsupported__";

pub fn unsup(why: impl Into<String>) -> Value {
    let mut m = Map::new();
    m.insert(UNSUPPORTED.into(), Value::String(why.into()));
    Value::Object(m)
}
pub fn is_unsup(v: &Value) -> bool {
    matches!(v, Value::Object(m) if m.contains_key(UNSUPPORTED))
}

/// A float as JSON: finite floats are numbers; NaN/Infinity become the strings Newtonsoft writes as bare tokens
/// ("NaN", "Infinity", "-Infinity"; serde_json cannot hold them as numbers)
pub fn fnum(x: f64) -> Value {
    if x.is_finite() {
        Value::from(x)
    } else if x.is_nan() {
        Value::from("NaN")
    } else if x > 0.0 {
        Value::from("Infinity")
    } else {
        Value::from("-Infinity")
    }
}

/// An object reference (CUE4Parse ResolvedObject): this package itself, one of its exports, or an unresolved import
#[derive(Clone)]
pub enum Node {
    Pkg(Rc<Package>),
    Export(Rc<Package>, usize),
    Import(Rc<Package>, usize),
}

/// Type data of an FPropertyTag (FPropertyTagData.cs:28-62)
#[derive(Default, Clone)]
struct TagData {
    struct_name: String,
    bool_val: u8,
    enum_name: String,
    inner: String,
    value: String,
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Normal, // a tagged property
    Array,  // an array element
    Map,    // a set/map element (CUE4Parse ReadType NORMAL/ARRAY/MAP)
}

pub struct Reader {
    pub vfs: Arc<Vfs>,
    cache: RefCell<HashMap<String, Option<Rc<Package>>>>,
    plugin_roots: RefCell<Option<HashMap<String, String>>>,
}

impl Reader {
    pub fn new(vfs: Arc<Vfs>) -> Reader {
        Reader { vfs, cache: RefCell::new(HashMap::new()), plugin_roots: RefCell::new(None) }
    }

    /// Cached package by path ("Mordhau/Content/.../BP_X", any case; a trailing ".N" export index is dropped).
    /// None if missing or not a 4.26 cook (`try_open` gives the reason).
    pub fn open(&self, pkg_path: &str) -> Option<Rc<Package>> {
        let p = strip_index(pkg_path);
        let k = p.to_lowercase();
        if let Some(v) = self.cache.borrow().get(&k) {
            return v.clone();
        }
        let a = Package::open(&self.vfs, p).ok().map(Rc::new);
        let mut c = self.cache.borrow_mut();
        if c.len() >= CACHE_MAX {
            c.clear();
        }
        c.insert(k, a.clone());
        a
    }
    pub fn try_open(&self, pkg_path: &str) -> Result<Rc<Package>, AssetError> {
        self.open(pkg_path).ok_or_else(|| match Package::open(&self.vfs, strip_index(pkg_path)) {
            Err(e) => e,
            Ok(_) => AssetError("unreachable".into()),
        })
    }
    pub fn clear_cache(&self) {
        self.cache.borrow_mut().clear();
    }

    /// Exports of a package in mdx json's shape; None when the package is not in the paks (UePkgPak.read)
    pub fn read(&self, pkg_path: &str) -> Option<Vec<Value>> {
        let a = self.open(pkg_path)?;
        Some((0..a.exports.len()).map(|i| self.export_json(&a, i)).collect())
    }

    // ---- object references (CUE4Parse Package.ResolvePackageIndex, Package.cs:332-426; ResolvedObject,
    // AbstractUePackage.cs) ----

    pub fn node(&self, pk: &Rc<Package>, idx: i32) -> Option<Node> {
        if idx == 0 {
            return None;
        }
        if idx > pk.exports.len() as i32 || -(idx as i64) > pk.imports.len() as i64 {
            pk.unsupported.borrow_mut().push(format!(
                "package index {idx} out of range ({} exports, {} imports)",
                pk.exports.len(),
                pk.imports.len()
            ));
            return None;
        }
        if idx > 0 {
            return Some(Node::Export(pk.clone(), (idx - 1) as usize));
        }
        Some(self.resolve_import(pk, (-idx - 1) as usize))
    }

    /// An import whose outermost package is a content package resolves to that package's export (matched by name and
    /// outer path), so ObjectPath carries the target's export index; /Script imports stay imports (Package.cs:343-426).
    fn resolve_import(&self, pk: &Rc<Package>, ii: usize) -> Node {
        let unresolved = || Node::Import(pk.clone(), ii);
        let im = &pk.imports[ii];
        let mut o = -(ii as i32) - 1;
        loop {
            if o > 0 {
                return unresolved();
            }
            let Some(om) = pk.imports.get((-o - 1) as usize) else { return unresolved() };
            if om.outer == 0 {
                break;
            }
            o = om.outer;
        }
        let top = &pk.imports[(-o - 1) as usize].name;
        if top.starts_with("/Script/") {
            return unresolved();
        }
        let target = self.package_path(top);
        let Some(ip) = (if target.is_empty() { None } else { self.open(&target) }) else { return unresolved() };
        let outer = if o != im.outer && im.outer < 0 {
            let on = self.resolve_import(pk, (-im.outer - 1) as usize);
            Some(self.path_name(&on, true))
        } else {
            None
        };
        for (i, e) in ip.exports.iter().enumerate() {
            if e.name != im.name {
                continue;
            }
            let to = self.node(&ip, e.outer).map(|n| self.path_name(&n, true));
            if to == outer {
                return Node::Export(ip.clone(), i);
            }
        }
        unresolved()
    }

    /// "/Game/X" -> "Mordhau/Content/X", "/Engine/X" -> "Engine/Content/X", "/<Plugin>/X" -> "<plugin dir>/Content/X"
    /// (the mount table UE builds from the project and its .uplugin files; CUE4Parse FixPath does the same)
    pub fn package_path(&self, game_path: &str) -> String {
        if let Some(s) = game_path.strip_prefix("/Game/") {
            return format!("Mordhau/Content/{s}");
        }
        if let Some(s) = game_path.strip_prefix("/Engine/") {
            return format!("Engine/Content/{s}");
        }
        let mut pr = self.plugin_roots.borrow_mut();
        let roots = pr.get_or_insert_with(|| {
            let mut m = HashMap::new();
            for f in self.vfs.list() {
                if let Some(c) = f.find("/Content/") {
                    if c > 0 && f.contains("/Plugins/") {
                        let root = &f[..c];
                        let last = root.rsplit('/').next().unwrap_or(root).to_lowercase();
                        m.insert(last, format!("{root}/Content/"));
                    }
                }
            }
            m
        });
        let gp = game_path.strip_prefix('/').unwrap_or(game_path);
        let seg = gp.split('/').next().unwrap_or("").to_lowercase();
        match roots.get(&seg) {
            Some(root) if gp.len() > seg.len() => format!("{root}{}", &gp[seg.len() + 1..]),
            Some(root) => root.clone(),
            None => String::new(),
        }
    }

    pub fn node_name(&self, n: &Node) -> String {
        match n {
            Node::Export(p, i) => p.exports[*i].name.clone(),
            Node::Import(p, i) => p.imports[*i].name.clone(),
            Node::Pkg(p) => p.name.clone(),
        }
    }

    pub fn node_outer(&self, n: &Node) -> Option<Node> {
        match n {
            Node::Export(p, i) => Some(self.node(p, p.exports[*i].outer).unwrap_or_else(|| Node::Pkg(p.clone()))),
            Node::Import(p, i) => self.node(p, p.imports[*i].outer),
            Node::Pkg(_) => None,
        }
    }

    pub fn node_class_name(&self, n: &Node) -> String {
        match n {
            Node::Export(p, i) => self.node(p, p.exports[*i].cls).map(|c| self.node_name(&c)).unwrap_or_default(),
            Node::Import(p, i) => p.imports[*i].class_name.clone(),
            Node::Pkg(_) => String::new(),
        }
    }

    /// ResolvedObject.GetPathName (AbstractUePackage.cs:181-202): outer chain joined by "." with ":" after a
    /// top-level object
    pub fn path_name(&self, n: &Node, incl_outermost: bool) -> String {
        let mut s = String::new();
        if let Some(o) = self.node_outer(n) {
            let oo = self.node_outer(&o);
            if oo.is_some() || incl_outermost {
                let colon = matches!(&oo, Some(x) if self.node_outer(x).is_none());
                s = self.path_name(&o, incl_outermost) + if colon { ":" } else { "." };
            }
        }
        s + &self.node_name(n)
    }

    pub fn full_name(&self, n: &Node, incl_outermost: bool) -> String {
        format!("{}'{}'", self.node_class_name(n), self.path_name(n, incl_outermost))
    }

    /// {ObjectName, ObjectPath} as CUE4Parse's ResolvedObjectConverter writes it (JsonConverters.cs:2874-2902)
    pub fn obj_json(&self, n: Option<Node>) -> Value {
        let Some(n) = n else { return Value::Null };
        let mut top = n.clone();
        while let Some(o) = self.node_outer(&top) {
            top = o;
        }
        let on = self.node_name(&top);
        let path = match &n {
            Node::Export(_, i) => format!("{on}.{i}"),
            Node::Import(..) | Node::Pkg(_) => on,
        };
        json!({"ObjectName": self.full_name(&n, false), "ObjectPath": path})
    }

    // ---- exports ----

    /// One export as mdx json writes it (UObject.WriteJson, UObject.cs:485-535): Type, Name, Class, Outer|Package,
    /// Super, Template, Properties; plus SuperStruct for UStruct exports, Rows for DataTables, Names/CppForm for enums,
    /// ReferenceSkeleton for skeletons.
    pub fn export_json(&self, pk: &Rc<Package>, i: usize) -> Value {
        let e = &pk.exports[i];
        let cn = self.node(pk, e.cls);
        let ty = cn.as_ref().map(|c| self.node_name(c)).unwrap_or_default();
        let mut d = Map::new();
        d.insert("Type".into(), Value::from(ty.clone()));
        d.insert("Name".into(), Value::from(e.name.clone()));
        match &cn {
            Some(c @ Node::Export(..)) => {
                d.insert("Class".into(), Value::from(self.full_name(c, true)));
            }
            Some(c @ Node::Import(p, ii)) if matches!(p.imports[*ii].class_name.as_str(), "Class" | "ScriptStruct") => {
                d.insert("Class".into(), Value::from(format!("UScriptClass'{}'", self.node_name(c))));
            }
            _ => {}
        }
        if e.outer != 0 {
            d.insert("Outer".into(), self.obj_json(self.node(pk, e.outer)));
        } else {
            d.insert("Package".into(), Value::from(pk.name.clone()));
        }
        for (k, idx) in [("Super", e.sup), ("Template", e.tmpl)] {
            if let Some(sn @ Node::Export(..)) = self.node(pk, idx) {
                d.insert(k.into(), self.obj_json(Some(sn)));
            }
        }
        let mut r = pk.cursor();
        r.p = e.off;
        let end = e.off + e.size;
        let props = self.tagged(pk, &mut r, end);
        if !props.is_empty() {
            d.insert("Properties".into(), Value::Object(props));
        }
        let guid = |r: &mut Cursor| {
            // UObject::Serialize: non-CDO objects then store bool bHasGuid (+ FGuid) (UObject.cs:226-236)
            if e.flags & RF_CLASS_DEFAULT_OBJECT == 0 && r.s32() != 0 {
                r.skip(16);
            }
        };
        if STRUCT_TYPES.contains(&ty.as_str()) || ty == "DataTable" {
            guid(&mut r);
            if ty == "DataTable" {
                // UDataTable::LoadStructData: int32 NumRows, then FName RowName + the row struct's tagged properties
                // (UDataTable.cs:50-56)
                let mut rows = Map::new();
                for _ in 0..count(&mut r, end) {
                    let rn = pk.fname(&mut r);
                    let row = self.tagged(pk, &mut r, end);
                    rows.insert(rn, Value::Object(row));
                }
                d.insert("Rows".into(), Value::Object(rows));
            } else {
                // UField (no Next since FFrameworkObjectVersion::RemoveUField_Next, 4.25) then UStruct::SuperStruct
                // (UStruct.cs:22)
                let idx = r.s32();
                let sup = self.obj_json(self.node(pk, idx));
                if !sup.is_null() {
                    d.insert("SuperStruct".into(), sup);
                }
            }
        } else if ENUM_TYPES.contains(&ty.as_str()) {
            // UObject Guid, UField (no Next, 4.25+), then UEnum::Serialize 4.26: TArray<TPair<FName, int64>> Names,
            // uint8 CppForm (UEnum.cs: Names via ReadArray(ReadFName, Read<long>), CppForm Read<ECppForm>)
            guid(&mut r);
            let mut names = Map::new();
            for _ in 0..count(&mut r, end) {
                let nm = pk.fname(&mut r);
                names.insert(nm, Value::from(r.s64()));
            }
            d.insert("Names".into(), Value::Object(names));
            let form = r.u8() as usize;
            d.insert("CppForm".into(), Value::from(CPP_FORM.get(form).map_or_else(|| form.to_string(), |s| s.to_string())));
        } else if ty == "Skeleton" {
            // UObject Guid, then USkeleton::Serialize (USkeleton.cs:44-47): the FReferenceSkeleton; the fields after it
            // (AnimRetargetSources, Guid, NameMappings) are not read.
            guid(&mut r);
            d.insert("ReferenceSkeleton".into(), self.ref_skeleton(pk, &mut r, end));
        } else if ty == "PhysicsAsset" {
            // Pinned CUE4Parse Objects/PhysicsEngine/UPhysicsAsset.cs: UObject tail, then
            // TMap<FRigidBodyIndexPair,bool>; FArchive bool occupies four bytes.
            guid(&mut r);
            let mut pairs = Vec::new();
            for _ in 0..count(&mut r, end) {
                let indices = [r.s32(), r.s32()];
                let value = r.s32() != 0;
                pairs.push(json!({"Key": {"Indices": indices}, "Value": value}));
            }
            d.insert("CollisionDisableTable".into(), Value::Array(pairs));
        }
        if r.p > end {
            pk.unsupported.borrow_mut().push(format!("export {} read past its end", e.name));
        }
        Value::Object(d)
    }

    /// FReferenceSkeleton (FReferenceSkeleton.cs:15-31, FMeshBoneInfo.cs:17-30): TArray<{FName Name; int32 ParentIndex}>
    /// FinalRefBoneInfo, TArray<FTransform {FQuat, FVector, FVector}> FinalRefBonePose, TMap<FName, int32>
    /// FinalNameToIndexMap; in a Skeleton export and in a SkeletalMesh's header
    pub fn ref_skeleton(&self, pk: &Rc<Package>, r: &mut Cursor, end: i64) -> Value {
        let mut info = Vec::new();
        for _ in 0..count(r, end) {
            let n = pk.fname(r);
            info.push(json!({"Name": n, "ParentIndex": r.s32()}));
        }
        let mut pose = Vec::new();
        for _ in 0..count(r, end) {
            let q = self.native_struct(pk, r, "Quat", 16, Mode::Normal, end);
            let t = self.native_struct(pk, r, "Vector", 12, Mode::Normal, end);
            let s = self.native_struct(pk, r, "Vector", 12, Mode::Normal, end);
            pose.push(json!({"Rotation": q, "Translation": t, "Scale3D": s}));
        }
        let mut idx = Map::new();
        for _ in 0..count(r, end) {
            let n = pk.fname(r);
            idx.insert(n, Value::from(r.s32()));
        }
        json!({"FinalRefBoneInfo": info, "FinalRefBonePose": pose, "FinalNameToIndexMap": idx})
    }

    // ---- tagged properties (FPropertyTag, UE4 branch: FPropertyTag.cs:186-205; FPropertyTagData.cs:28-62) ----

    /// Read tags until "None". Each value is decoded by type; a value this reader cannot decode is recorded in
    /// `unsupported` and skipped by the tag's Size, so one unknown struct never desyncs the rest (CUE4Parse does the
    /// same, FPropertyTag.cs:207-238). `end` bounds a nested struct (its tag Size, or the enclosing property's end): a
    /// tag that does not fit there means the bytes are not tagged properties (a native struct the tag did not name),
    /// reported as "__unsupported__".
    pub fn tagged(&self, pk: &Rc<Package>, r: &mut Cursor, end: i64) -> Map<String, Value> {
        let mut out = Map::new();
        let limit = if end < 0 { r.len() } else { end.min(r.len()) };
        while r.p + 8 <= limit {
            let pname = pk.fname(r);
            if pname == "None" {
                break;
            }
            let typ = pk.fname(r);
            let size = r.s32();
            let aidx = r.s32();
            let mut td = TagData::default();
            match typ.as_str() {
                "StructProperty" => {
                    td.struct_name = pk.fname(r);
                    r.skip(16); // StructGuid
                }
                "BoolProperty" => td.bool_val = r.u8(), // the value lives in the tag (PROPERTYTAG_BOOL_OPTIMIZATION)
                "ByteProperty" | "EnumProperty" => td.enum_name = pk.fname(r),
                "ArrayProperty" | "SetProperty" => td.inner = pk.fname(r),
                "MapProperty" => {
                    td.inner = pk.fname(r);
                    td.value = pk.fname(r);
                }
                _ => {}
            }
            if r.u8() != 0 {
                r.skip(16); // HasPropertyGuid
            }
            let start = r.p;
            if r.bad || size < 0 || start + size as i64 > limit {
                r.bad = false;
                out.insert(UNSUPPORTED.into(), Value::from(format!("no tag at {start}")));
                return out;
            }
            let key = if aidx == 0 { pname } else { format!("{pname}[{aidx}]") };
            let vend = start + size as i64;
            let mut v = self.value(pk, r, &typ, &td, size, Mode::Normal, vend);
            if r.bad {
                r.bad = false;
                v = unsup("read past the data");
            }
            if is_unsup(&v) {
                let why = v[UNSUPPORTED].as_str().unwrap_or("").to_string();
                pk.unsupported.borrow_mut().push(format!("{typ} {key}: {why}"));
            } else if r.p != vend {
                pk.unsupported.borrow_mut().push(format!("{typ} {key}: read {} of {size} bytes", r.p - start));
                v = unsup("size");
            }
            out.insert(key, v);
            r.p = vend;
        }
        out
    }

    #[allow(clippy::too_many_arguments)]
    fn value(&self, pk: &Rc<Package>, r: &mut Cursor, typ: &str, td: &TagData, size: i32, mode: Mode, end: i64) -> Value {
        match typ {
            "BoolProperty" => Value::Bool(if mode == Mode::Normal { td.bool_val != 0 } else { r.u8() != 0 }),
            "IntProperty" => Value::from(r.s32()),
            "Int8Property" => Value::from(r.s8()),
            "Int16Property" => Value::from(r.s16()),
            "Int64Property" => Value::from(r.s64()),
            "UInt16Property" => Value::from(r.u16()),
            "UInt32Property" => Value::from(r.u32()),
            "UInt64Property" => Value::from(r.u64()),
            "FloatProperty" => fnum(r.f32()),
            "DoubleProperty" => fnum(r.f64()),
            "NameProperty" | "EnumProperty" => Value::from(pk.fname(r)),
            "StrProperty" => Value::from(r.fstring()),
            "TextProperty" => self.text(pk, r),
            t if OBJECT_PROPS.contains(&t) => {
                let i = r.s32();
                self.obj_json(self.node(pk, i))
            }
            "SoftObjectProperty" | "SoftClassProperty" | "AssetObjectProperty" | "AssetClassProperty" => soft_path(pk, r),
            // TLazyObjectPtr is serialized as FUniqueObjectGuid (one FGuid), written {"Guid": ...}
            // (LazyObjectProperty.cs:12-19, FUniqueObjectGuid.cs:7-10, JsonConverters.cs:559-564). ue_asset.gd read it as a
            // package index, a 16-byte value read as 4 bytes: "size" (the LandscapeComponent CollisionComponent and
            // LandscapeHeightfieldCollisionComponent RenderComponent links)
            "LazyObjectProperty" => {
                let g = self.native_struct(pk, r, "Guid", 16, mode, end);
                json!({ "Guid": g })
            }
            "ByteProperty" => {
                if mode == Mode::Normal {
                    return if td.enum_name.is_empty() || td.enum_name == "None" {
                        Value::from(r.u8())
                    } else {
                        Value::from(pk.fname(r))
                    };
                }
                // A map's byte key/value has no enum name in the tag; UE knows it from the FByteProperty. Without the
                // class schema, read an FName when the next 8 bytes are a valid name reference, as CUE4Parse does
                // (FPropertyTagType.cs:155, FAssetArchive.TestReadFName FAssetArchive.cs:61-70)
                if mode == Mode::Map && r.p + 8 < end {
                    if let (Some(ni), Some(nn)) = (r.peek_s32(0), r.peek_s32(4)) {
                        if ni >= 0 && (ni as usize) < pk.names.len() && (0..256).contains(&nn) {
                            return Value::from(pk.fname(r));
                        }
                    }
                }
                Value::from(r.u8())
            }
            "StructProperty" => self.native_struct(pk, r, &td.struct_name, size, mode, end),
            "ArrayProperty" => {
                let n = r.s32();
                if n < 0 || n as i64 > end - r.p {
                    return unsup(format!("array count {n}"));
                }
                let inner = td.inner.as_str();
                let mut itd = TagData::default();
                if inner == "StructProperty" {
                    // INNER_ARRAY_TAG_INFO: struct arrays carry one inner FPropertyTag (UScriptArray.cs:51-56)
                    pk.fname(r);
                    pk.fname(r);
                    r.skip(8);
                    itd.struct_name = pk.fname(r);
                    r.skip(16);
                    if r.u8() != 0 {
                        r.skip(16);
                    }
                }
                let mut a = Vec::with_capacity(n as usize);
                if inner == "ByteProperty" && n > 0 {
                    // enum arrays store FNames: element size > 1 byte (UScriptArray.cs:72-84)
                    let as_name = size as i64 - 4 >= 2 * n as i64;
                    for _ in 0..n {
                        a.push(if as_name { Value::from(pk.fname(r)) } else { Value::from(r.u8()) });
                    }
                    return Value::Array(a);
                }
                for _ in 0..n {
                    let v = self.value(pk, r, inner, &itd, 0, Mode::Array, end);
                    if is_unsup(&v) {
                        return v;
                    }
                    a.push(v);
                }
                Value::Array(a)
            }
            "SetProperty" => {
                let none = TagData::default();
                for _ in 0..count(r, end) {
                    self.value(pk, r, &td.inner, &none, 0, Mode::Map, end); // NumElementsToRemove
                }
                let mut a = Vec::new();
                for _ in 0..count(r, end) {
                    let v = self.value(pk, r, &td.inner, &none, 0, Mode::Map, end);
                    if r.bad || is_unsup(&v) {
                        return unsup("set element");
                    }
                    a.push(v);
                }
                Value::Array(a)
            }
            "MapProperty" => {
                // NumKeysToRemove (+ keys), NumEntries, then key/value pairs (UScriptMap.cs:54-82). Struct keys/values
                // carry no struct name in UE4 tags; they are read as tagged properties (UScriptStruct::SerializeItem
                // default)
                let none = TagData::default();
                for _ in 0..count(r, end) {
                    self.value(pk, r, &td.inner, &none, 0, Mode::Map, end);
                }
                let mut a = Vec::new();
                for _ in 0..count(r, end) {
                    let k = self.map_key(pk, r, &td.inner, end);
                    let v = self.value(pk, r, &td.value, &none, 0, Mode::Map, end);
                    if r.bad || is_unsup(&k) || is_unsup(&v) {
                        return unsup("map entry");
                    }
                    a.push(json!({"Key": k, "Value": v}));
                }
                Value::Array(a)
            }
            "MulticastDelegateProperty" | "MulticastInlineDelegateProperty" | "MulticastSparseDelegateProperty" => {
                let mut inv = Vec::new();
                for _ in 0..count(r, end) {
                    let i = r.s32();
                    let o = self.obj_json(self.node(pk, i));
                    inv.push(json!({"Object": o, "FunctionName": pk.fname(r)}));
                }
                json!({ "InvocationList": inv })
            }
            "FieldPathProperty" => {
                // FFieldPath: TArray<FName> Path (["None"] = empty) + FPackageIndex ResolvedOwner (FFieldPath.cs:18-30;
                // UE FieldPath.h, owner serialized since FReleaseObjectVersion::FFieldPathOwnerSerialization)
                let mut path = Vec::new();
                for _ in 0..count(r, end) {
                    path.push(pk.fname(r));
                }
                if path.len() == 1 && path[0] == "None" {
                    path.clear();
                }
                let i = r.s32();
                json!({"Path": path, "ResolvedOwner": self.obj_json(self.node(pk, i))})
            }
            "DelegateProperty" => {
                let i = r.s32();
                let o = self.obj_json(self.node(pk, i));
                json!({"Object": o, "FunctionName": pk.fname(r)})
            }
            _ => unsup(format!("type {typ}")),
        }
    }

    /// A map key that is not a struct is written by CUE4Parse as the key's ToString() up to the first "("
    /// (JsonConverters.cs:1058-1088): an object as GetFullName() including its package ("Class'Pkg.Obj'"), "0" for null.
    fn map_key(&self, pk: &Rc<Package>, r: &mut Cursor, inner: &str, end: i64) -> Value {
        if OBJECT_PROPS.contains(&inner) {
            let i = r.s32();
            return match self.node(pk, i) {
                None => Value::from("0"),
                Some(n) => Value::from(cut_paren(&self.full_name(&n, true))),
            };
        }
        let k = self.value(pk, r, inner, &TagData::default(), 0, Mode::Map, end);
        if inner == "StructProperty" || is_unsup(&k) {
            return k;
        }
        let s = match &k {
            Value::Object(m) if m.contains_key("AssetPathName") => {
                let a = m["AssetPathName"].as_str().unwrap_or("");
                let sub = m.get("SubPathString").and_then(|v| v.as_str()).unwrap_or("");
                if sub.is_empty() {
                    if a == "None" { String::new() } else { a.to_string() }
                } else {
                    format!("{a}:{sub}")
                }
            }
            Value::Bool(b) => (if *b { "True" } else { "False" }).to_string(),
            Value::String(s) => s.clone(),
            // .NET ToString of a number: shortest round-trip, no trailing ".0"
            Value::Number(n) => n.as_f64().filter(|_| n.is_f64()).map_or_else(|| n.to_string(), |f| format!("{}", f as f32)),
            other => other.to_string(),
        };
        Value::from(cut_paren(&s))
    }

    fn native_struct(&self, pk: &Rc<Package>, r: &mut Cursor, st: &str, size: i32, mode: Mode, end: i64) -> Value {
        match st {
            "Vector" => json!({"X": fnum(r.f32()), "Y": fnum(r.f32()), "Z": fnum(r.f32())}),
            "Vector2D" => json!({"X": fnum(r.f32()), "Y": fnum(r.f32())}),
            "Vector4" | "Quat" => json!({"X": fnum(r.f32()), "Y": fnum(r.f32()), "Z": fnum(r.f32()), "W": fnum(r.f32())}),
            "Rotator" => json!({"Pitch": fnum(r.f32()), "Yaw": fnum(r.f32()), "Roll": fnum(r.f32())}),
            "LinearColor" => json!({"R": fnum(r.f32()), "G": fnum(r.f32()), "B": fnum(r.f32()), "A": fnum(r.f32())}),
            "Color" => json!({"B": r.u8(), "G": r.u8(), "R": r.u8(), "A": r.u8()}), // FColor is stored BGRA
            "IntPoint" => json!({"X": r.s32(), "Y": r.s32()}),
            "IntVector" => json!({"X": r.s32(), "Y": r.s32(), "Z": r.s32()}),
            // FGuid A,B,C,D (uint32); printed EGuidFormats::UniqueObjectGuid, as the JSON has it (JsonConverters.cs:2961-2966)
            "Guid" => Value::from(format!("{:08X}-{:08X}-{:08X}-{:08X}", r.u32(), r.u32(), r.u32(), r.u32())),
            "Box" => {
                let mn = self.native_struct(pk, r, "Vector", 12, mode, end);
                let mx = self.native_struct(pk, r, "Vector", 12, mode, end);
                json!({"Min": mn, "Max": mx, "IsValid": r.u8()})
            }
            "Box2D" => {
                let mn = self.native_struct(pk, r, "Vector2D", 8, mode, end);
                let mx = self.native_struct(pk, r, "Vector2D", 8, mode, end);
                json!({"Min": mn, "Max": mx, "bIsValid": r.u8()})
            }
            "FrameNumber" => json!({"Value": r.s32()}),
            "MovieSceneFrameRange" => {
                // TRange<FFrameNumber>: LowerBound then UpperBound, each TRangeBound {uint8 Type (ERangeBoundTypes);
                // FFrameNumber Value} (FMovieSceneFrameRange.cs, TRangeBound.cs:24-36); 5 bytes per bound as UE
                // serializes it, 8 when the tag's size says the bound was written with its in-memory padding
                let pad = if size == 16 { 3 } else { 0 };
                let lt = r.u8();
                r.skip(pad);
                let lv = r.s32();
                let ht = r.u8();
                r.skip(pad);
                let hv = r.s32();
                json!({"Value": {"LowerBound": {"Type": lt, "Value": {"Value": lv}},
                                 "UpperBound": {"Type": ht, "Value": {"Value": hv}}}})
            }
            "FontData" => {
                // FFontData::Serialize, cooked (FFontData.cs:27-40): bool bIsCooked, then FPackageIndex
                // LocalFontFaceAsset; a null asset (font by file name) is not decoded; then int32 SubFaceIndex
                if r.s32() == 0 {
                    return unsup("uncooked FontData");
                }
                let i = r.s32();
                let face = self.obj_json(self.node(pk, i));
                if face.is_null() {
                    return unsup("FontData by file name");
                }
                json!({"LocalFontFaceAsset": face, "SubFaceIndex": r.s32()})
            }
            // FNavAgentSelector: uint32 PackedBits (AI/Navigation/NavigationTypes.h)
            "NavAgentSelector" => json!({"PackedBits": r.u32()}),
            "DateTime" | "Timespan" => json!({"Ticks": r.s64()}),
            "GameplayTagContainer" => {
                let mut a = Vec::new();
                for _ in 0..count(r, end) {
                    a.push(Value::from(pk.fname(r)));
                }
                Value::Array(a)
            }
            "RichCurveKey" => {
                let (im, tm, tw) = (r.u8() as usize, r.u8() as usize, r.u8() as usize);
                let nm = |t: &[&str], i: usize| t.get(i).map_or_else(|| i.to_string(), |s| s.to_string());
                json!({"InterpMode": nm(RCIM, im), "TangentMode": nm(RCTM, tm), "TangentWeightMode": nm(RCTWM, tw),
                    "Time": fnum(r.f32()), "Value": fnum(r.f32()), "ArriveTangent": fnum(r.f32()),
                    "ArriveTangentWeight": fnum(r.f32()), "LeaveTangent": fnum(r.f32()), "LeaveTangentWeight": fnum(r.f32())})
            }
            s if SOFT_PATH_STRUCTS.contains(&s) => soft_path(pk, r),
            s if NET_QUANTIZE.contains(&s) => {
                // FVector_NetQuantize* derive from FVector, but these packages store them as tagged structs (Size != 12);
                // CUE4Parse reads 12 raw bytes regardless (FScriptStruct.cs:192-195) and so misreads them.
                if mode == Mode::Normal && size != 12 {
                    Value::Object(self.tagged(pk, r, end))
                } else {
                    self.native_struct(pk, r, "Vector", 12, mode, end)
                }
            }
            // FExpressionInput / FMaterialInput<T>, cooked native serialize (UE 4.26 MaterialShared.cpp
            // FExpressionInput::Serialize + FMaterialInput<T>::Serialize; CUE4Parse FExpressionInput.cs, same field
            // order as extract/json): int32 OutputIndex, FName InputName, int32 Mask, MaskR, MaskG, MaskB, MaskA,
            // FName ExpressionName [, bool32 UseConstant, T Constant]. The Expression pointer is editor-only (null).
            // Decoded only when the tag size equals the layout's (40 + constant), else reported unsupported.
            "ExpressionInput" | "ColorMaterialInput" | "ScalarMaterialInput" | "VectorMaterialInput" | "Vector2MaterialInput" => {
                let constant = match st {
                    "ColorMaterialInput" => 4,
                    "ScalarMaterialInput" => 4,
                    "VectorMaterialInput" => 12,
                    "Vector2MaterialInput" => 8,
                    _ => 0,
                };
                let want = 40 + if constant > 0 { 4 + constant } else { 0 };
                if mode != Mode::Normal || size != want {
                    return unsup(format!("native struct {st} size {size}"));
                }
                let mut o = serde_json::Map::new();
                o.insert("Expression".into(), Value::Null);
                o.insert("OutputIndex".into(), json!(r.s32()));
                o.insert("InputName".into(), Value::from(pk.fname(r)));
                for k in ["Mask", "MaskR", "MaskG", "MaskB", "MaskA"] {
                    o.insert(k.into(), json!(r.s32()));
                }
                o.insert("ExpressionName".into(), Value::from(pk.fname(r)));
                if constant > 0 {
                    o.insert("UseConstant".into(), json!(r.s32() != 0));
                    let c = match st {
                        "ColorMaterialInput" => self.native_struct(pk, r, "Color", 4, mode, end),
                        "ScalarMaterialInput" => fnum(r.f32()),
                        "VectorMaterialInput" => self.native_struct(pk, r, "Vector", 12, mode, end),
                        _ => self.native_struct(pk, r, "Vector2D", 8, mode, end),
                    };
                    o.insert("Constant".into(), c);
                }
                Value::Object(o)
            }
            s if BINARY_STRUCTS.contains(&s) => unsup(format!("native struct {s}")),
            _ => Value::Object(self.tagged(pk, r, end)),
        }
    }

    /// FText (FText.cs:116-160; UE TextHistory.cpp): uint32 Flags, int8 HistoryType, then the history
    fn text(&self, pk: &Rc<Package>, r: &mut Cursor) -> Value {
        r.skip(4);
        match r.s8() {
            // None: bHasCultureInvariantString + string (FText.cs:188-198)
            -1 => {
                let s = if r.s32() != 0 { Value::from(r.fstring()) } else { Value::Null };
                json!({ "CultureInvariantString": s })
            }
            // Base: Namespace, Key, SourceString (FText.cs:208-213). LocalizedString = SourceString: no .locres is
            // applied (mdx json does not load one either)
            0 => {
                let ns = r.fstring();
                let key = r.fstring();
                let src = r.fstring();
                json!({"Namespace": ns, "Key": key, "SourceString": src, "LocalizedString": src})
            }
            // StringTableEntry: FName TableId, FString Key
            11 => {
                let tid = pk.fname(r);
                json!({"TableId": tid, "Key": r.fstring()})
            }
            ht => unsup(format!("text history {ht}")),
        }
    }
}

/// An element count that cannot be right (negative, or more elements than bytes left) reads as 0 and marks the cursor bad
pub(crate) fn count(r: &mut Cursor, end: i64) -> i32 {
    let n = r.s32();
    if n < 0 || n as i64 > end - r.p {
        r.bad = true;
        return 0;
    }
    n
}

/// FSoftObjectPath: FName AssetPathName + FString SubPathString (FSoftObjectPath.cs; UE SoftObjectPath.cpp SerializePath)
fn soft_path(pk: &Package, r: &mut Cursor) -> Value {
    let a = pk.fname(r);
    json!({"AssetPathName": a, "SubPathString": r.fstring()})
}

fn cut_paren(s: &str) -> String {
    s.split('(').next().unwrap_or("").trim().to_string()
}

/// "Mordhau/Content/.../BP_X.0" -> "Mordhau/Content/.../BP_X" (mdx json appends the export index)
pub fn strip_index(p: &str) -> &str {
    match p.rsplit_once('.') {
        Some((a, b)) if !b.is_empty() && !b.contains('/') && b.trim_start_matches(['-', '+']).chars().all(|c| c.is_ascii_digit()) && !b.trim_start_matches(['-', '+']).is_empty() => a,
        _ => p,
    }
}

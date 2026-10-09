//! Tagged properties (FPropertyTag, UE4 branch: CUE4Parse FPropertyTag.cs:186-205, FPropertyTagData.cs:28-62), read
//! only as far as the asset decoders need: every tag is walked (so the cursor lands on the native data after "None"),
//! and the values the decoders use are decoded - scalars, names/enums, strings, tagged structs and arrays of them
//! (e.g. a Skeleton's BoneTree). Anything else is kept as `Value::Skipped` and stepped over by the tag's Size, as
//! UeAsset.tagged does (godot/components/ue/pak/ue_asset.gd), so an undecoded value never desyncs the rest. mh-pak
//! owns the full property reader (verified against extract/json); this one is the decoders' private cursor.

use crate::buf::Buf;
use crate::RawPackage;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    /// NameProperty, EnumProperty, and ByteProperty with an enum ("EAdditiveAnimationType::AAT_None" style as stored)
    Name(String),
    Str(String),
    Struct(Props),
    Array(Vec<Value>),
    /// a value this reader does not decode, skipped by its tag size (type name kept)
    Skipped(String),
}

pub type Props = BTreeMap<String, Value>;

impl Value {
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Name(s) | Value::Str(s) => Some(s),
            _ => None,
        }
    }
}

pub fn get_bool(p: &Props, k: &str, default: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(default)
}
pub fn get_f64(p: &Props, k: &str, default: f64) -> f64 {
    p.get(k).and_then(Value::as_f64).unwrap_or(default)
}
pub fn get_str<'a>(p: &'a Props, k: &str, default: &'a str) -> &'a str {
    p.get(k).and_then(Value::as_str).unwrap_or(default)
}

struct TagData {
    struct_name: String,
    bool_val: u8,
    enum_name: String,
    inner: String,
}

/// Read tags from r.p until "None" (or `end`); the cursor is left after the terminating "None"
pub fn read_tagged(pkg: &RawPackage, r: &mut Buf, end: usize) -> Props {
    let mut out = Props::new();
    let limit = end.min(r.b.len());
    while r.p + 8 <= limit {
        let name = pkg.fname(r);
        if name == "None" {
            break;
        }
        let typ = pkg.fname(r);
        let size = r.s32();
        let aidx = r.s32();
        let mut td = TagData { struct_name: String::new(), bool_val: 0, enum_name: String::new(), inner: String::new() };
        match typ.as_str() {
            "StructProperty" => {
                td.struct_name = pkg.fname(r);
                r.skip(16); // StructGuid
            }
            "BoolProperty" => td.bool_val = r.u8(), // PROPERTYTAG_BOOL_OPTIMIZATION: the value lives in the tag
            "ByteProperty" | "EnumProperty" => td.enum_name = pkg.fname(r),
            "ArrayProperty" | "SetProperty" => td.inner = pkg.fname(r),
            "MapProperty" => {
                td.inner = pkg.fname(r);
                pkg.fname(r);
            }
            _ => {}
        }
        if r.u8() != 0 {
            r.skip(16); // HasPropertyGuid
        }
        let start = r.p;
        if r.bad || size < 0 || start + size as usize > limit {
            r.bad = false;
            r.p = limit;
            return out;
        }
        let vend = start + size as usize;
        let key = if aidx == 0 { name } else { format!("{}[{}]", name, aidx) };
        let mut v = value(pkg, r, &typ, &td, size as usize, vend);
        if r.bad || r.p != vend {
            r.bad = false;
            v = Value::Skipped(typ.clone());
        }
        out.insert(key, v);
        r.p = vend;
    }
    out
}

fn value(pkg: &RawPackage, r: &mut Buf, typ: &str, td: &TagData, size: usize, end: usize) -> Value {
    match typ {
        "BoolProperty" => Value::Bool(td.bool_val != 0),
        "IntProperty" => Value::Int(r.s32() as i64),
        "Int8Property" => Value::Int(r.s8() as i64),
        "Int16Property" => Value::Int(r.s16() as i64),
        "Int64Property" => Value::Int(r.s64()),
        "UInt16Property" => Value::Int(r.u16() as i64),
        "UInt32Property" => Value::Int(r.u32() as i64),
        "FloatProperty" => Value::Float(r.f32() as f64),
        "DoubleProperty" => Value::Float(r.f64()),
        "NameProperty" | "EnumProperty" => Value::Name(pkg.fname(r)),
        "StrProperty" => Value::Str(r.fstring()),
        "ByteProperty" => {
            if td.enum_name == "None" || td.enum_name.is_empty() {
                Value::Int(r.u8() as i64)
            } else {
                Value::Name(pkg.fname(r))
            }
        }
        "StructProperty" => strukt(pkg, r, &td.struct_name, end),
        "ArrayProperty" => {
            let n = r.s32();
            if n < 0 || n as usize > end.saturating_sub(r.p) {
                return Value::Skipped("ArrayProperty".into());
            }
            let mut itd = TagData { struct_name: String::new(), bool_val: 0, enum_name: String::new(), inner: String::new() };
            if td.inner == "StructProperty" {
                // INNER_ARRAY_TAG_INFO: struct arrays carry one inner FPropertyTag (UScriptArray.cs:51-56)
                pkg.fname(r);
                pkg.fname(r);
                r.skip(8);
                itd.struct_name = pkg.fname(r);
                r.skip(16);
                if r.u8() != 0 {
                    r.skip(16);
                }
            }
            let mut a = Vec::with_capacity(n as usize);
            if td.inner == "ByteProperty" {
                // enum arrays store FNames: element size > 1 byte (UScriptArray.cs:72-84)
                let as_name = size >= 4 + 2 * n as usize;
                for _ in 0..n {
                    a.push(if as_name { Value::Name(pkg.fname(r)) } else { Value::Int(r.u8() as i64) });
                }
                return Value::Array(a);
            }
            if td.inner == "BoolProperty" {
                for _ in 0..n {
                    a.push(Value::Bool(r.u8() != 0));
                }
                return Value::Array(a);
            }
            for _ in 0..n {
                let v = value(pkg, r, &td.inner, &itd, 0, end);
                if let Value::Skipped(_) = v {
                    return v;
                }
                if r.bad {
                    return Value::Skipped("ArrayProperty".into());
                }
                a.push(v);
            }
            Value::Array(a)
        }
        _ => Value::Skipped(typ.to_string()),
    }
}

/// Natively serialized structs whose layout the decoders use; every other struct name not in BINARY is tagged
/// (UScriptStruct::SerializeItem default; CUE4Parse FScriptStruct.cs:78-198)
fn strukt(pkg: &RawPackage, r: &mut Buf, st: &str, end: usize) -> Value {
    let f = |r: &mut Buf| Value::Float(r.f32() as f64);
    let mk = |pairs: Vec<(&str, Value)>| Value::Struct(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect());
    match st {
        "Vector" => mk(vec![("X", f(r)), ("Y", f(r)), ("Z", f(r))]),
        "Vector2D" => mk(vec![("X", f(r)), ("Y", f(r))]),
        "Vector4" | "Quat" => mk(vec![("X", f(r)), ("Y", f(r)), ("Z", f(r)), ("W", f(r))]),
        "Rotator" => mk(vec![("Pitch", f(r)), ("Yaw", f(r)), ("Roll", f(r))]),
        "LinearColor" => mk(vec![("R", f(r)), ("G", f(r)), ("B", f(r)), ("A", f(r))]),
        "Color" => {
            let b = r.u8();
            let g = r.u8();
            let rr = r.u8();
            let a = r.u8();
            mk(vec![("B", Value::Int(b as i64)), ("G", Value::Int(g as i64)), ("R", Value::Int(rr as i64)),
                ("A", Value::Int(a as i64))])
        }
        "IntPoint" => mk(vec![("X", Value::Int(r.s32() as i64)), ("Y", Value::Int(r.s32() as i64))]),
        "Guid" => {
            r.skip(16);
            Value::Skipped("Guid".into())
        }
        _ if BINARY.contains(&st) => Value::Skipped(st.to_string()),
        _ => Value::Struct(read_tagged(pkg, r, end)),
    }
}

/// Natively serialized structs not decoded here (skipped by tag size); list from UeAsset BINARY_STRUCTS +
/// the native ones it decodes but these decoders never read (ue_asset.gd:32-43, 622-680)
const BINARY: &[&str] = &[
    "Plane", "PerPlatformFloat", "PerPlatformInt", "PerPlatformBool", "SimpleCurveKey", "Sphere", "TwoVectors",
    "MaterialAttributesInput", "ExpressionInput", "ColorMaterialInput", "ScalarMaterialInput", "VectorMaterialInput",
    "Vector2MaterialInput", "SmartName", "FontCharacter", "UniqueNetIdRepl", "MovieSceneFloatChannel",
    "MovieSceneEvaluationKey", "SectionEvaluationDataTree", "LevelSequenceObjectReferenceMap", "MovieSceneSegment",
    "MovieSceneTrackIdentifier", "MovieSceneSequenceID", "MovieSceneEvalTemplatePtr",
    "MovieSceneTrackImplementationPtr", "NameCurveKey", "StringCurveKey", "SkeletalMeshSamplingLODBuiltData",
    "CompressedRichCurve", "MovieSceneSegmentIdentifier", "MovieSceneSubSequenceTree", "MovieSceneEventParameters",
    "MovieSceneSequenceInstanceDataPtr", "MovieSceneEvaluationFieldEntityTree", "MovieSceneFloatValue", "ClothLODData",
    "ClothTetherData", "NiagaraVariable", "NiagaraVariableBase", "NiagaraVariableWithOffset", "Transform3f",
    "MovieSceneTrackFieldData", "MovieSceneSubSectionFieldData", "SkeletalMeshSamplingRegionBuiltData",
    "PerPlatformFrameRate", "PerQualityLevelInt", "PerQualityLevelFloat", "IntVector", "Box", "Box2D", "FrameNumber",
    "MovieSceneFrameRange", "FontData", "NavAgentSelector", "DateTime", "Timespan", "GameplayTagContainer",
    "RichCurveKey", "SoftObjectPath", "SoftClassPath", "StringAssetReference", "StringClassReference",
    "Vector_NetQuantize", "Vector_NetQuantize10", "Vector_NetQuantize100", "Vector_NetQuantizeNormal",
];

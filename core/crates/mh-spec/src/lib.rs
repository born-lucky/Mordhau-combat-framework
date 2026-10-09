//! mh-spec: the Spreadsheet Method spec matrix (docs/methods/SPREADSHEET-METHOD.md) as typed Rust data.
//!
//! `data_gen/spec/` is generated (never hand-edited) by `tools/sheets/build_matrix.py` from the workbook
//! `sheets/mordhau_spec.xlsx`, which `scripts/sheets_populate.py` fills from cited sources (the components/ue record
//! readers over the user's own paks / extract/json and native constructor replay, the hash-pinned exe's .rdata,
//! Blueprint bytecode literals). Schema: docs/SPEC_SHEETS.md (format `mordhau-spec/1`). It is Triternion data and
//! stays local (`data_gen/` is git-ignored).
//!
//! Engine code looks values up by permanent ID (`ENT_*` entity, `FLD_*` field, `RULE_*` rule) and implements each
//! rule's shared semantics once (here: [`Spec::attack_for_move`] for `RULE_ATTACK_SLOT_BY_MOVE` +
//! `RULE_ALT_MODE_ATTACK_SWAP`). Every typed getter checks the field's declared type, so a wrong read is an error,
//! never a silent 0.
//!
//! Numbers: a `float` field is an IEEE binary32 value (UE `float`) written as its shortest round-trip decimal; read
//! it with [`Spec::f32`]. Converting that decimal to f64 is NOT the GDScript reference's f64 for constructor-sourced
//! values (0.33 vs 0.33000001311302185): compare as f32. `double` fields are f64.

use serde::Deserialize;
use serde_json::Value;
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "mordhau-spec/1";

/// Spec value types (docs/SPEC_SHEETS.md "Types").
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldType {
    Bool,
    Int,
    Float,
    Double,
    String,
    Ref,
    Vec2,
    Vec3,
    Color,
    FloatList,
    IntList,
    StringList,
    Json,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Field {
    pub entity_type: String,
    pub name: String,
    #[serde(rename = "type")]
    pub ty: FieldType,
    pub ref_type: String,
    pub unit: String,
    pub unit_source: String,
    pub source: String,
    pub description: String,
    /// evidence tiers the field has: "a" shipped-data, "b" exe-code, "c" port-tested, "d" demo-observed
    /// (docs/SPEC_SHEETS.md "Coverage"); empty = none
    #[serde(default)]
    pub tiers: Vec<String>,
    #[serde(default)]
    pub tier_note: String,
}

impl Field {
    pub fn has_tier(&self, t: &str) -> bool {
        self.tiers.iter().any(|x| x == t)
    }
}

#[derive(Clone, Debug, Deserialize)]
pub struct Entity {
    pub name: String,
    pub parent: Option<String>,
    pub source: String,
    pub record: String,
    /// field id -> value (absent = the field does not apply to this entity)
    pub values: BTreeMap<String, Value>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Rule {
    #[serde(rename = "type")]
    pub ty: String,
    pub name: String,
    pub description: String,
    pub params: Value,
    pub fields: Vec<String>,
    pub source: String,
    pub implemented_by: Vec<String>,
    pub area: String,
    pub tests: Vec<String>,
    pub status: String,
    /// evidence tiers (see `Field::tiers`)
    #[serde(default)]
    pub tiers: Vec<String>,
    #[serde(default)]
    pub tier_note: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Evidence {
    pub target_id: String,
    pub status: String,
    pub observation: String,
    pub source: String,
    pub version: String,
    pub procedure: String,
    pub expected: Value,
    pub observed: Value,
    pub tests: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Source {
    pub kind: String,
    pub path: String,
    pub cite: String,
    pub version: String,
}

#[derive(Clone, Debug, Deserialize)]
pub struct EntityTypeInfo {
    pub file: String,
    /// the type's ID code: every `ENT_<code>_*` / `FLD_<code>_*` of the type (build_matrix checks it)
    pub code: String,
    pub entities: usize,
    pub fields: usize,
    pub values: usize,
}

#[derive(Clone, Debug, Deserialize)]
pub struct Index {
    pub format: String,
    pub content_sha1: String,
    pub entity_types: BTreeMap<String, EntityTypeInfo>,
    pub counts: BTreeMap<String, usize>,
    pub evidence_status: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SpecError {
    Io(String),
    Parse(String),
    Format(String),
    UnknownEntity(String),
    UnknownField(String),
    UnknownRule(String),
    /// (entity, field): the field belongs to another entity type
    WrongEntityType(String, String),
    /// (field, declared, requested)
    WrongType(String, FieldType, &'static str),
    /// (entity, field): the field applies to this type but this entity has no value
    Unset(String, String),
    /// (entity, field, why)
    BadValue(String, String, String),
}

impl fmt::Display for SpecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for SpecError {}

pub type Result<T> = std::result::Result<T, SpecError>;

/// The loaded matrix.
#[derive(Clone, Debug)]
pub struct Spec {
    pub index: Index,
    pub fields: BTreeMap<String, Field>,
    pub entities: BTreeMap<String, Entity>,
    pub rules: BTreeMap<String, Rule>,
    pub sources: BTreeMap<String, Source>,
    /// empty unless loaded with `with_evidence`
    pub evidence: BTreeMap<String, Evidence>,
    /// entity id -> entity type
    types: BTreeMap<String, String>,
}

fn parse<T: for<'a> Deserialize<'a>>(what: &str, text: &str) -> Result<T> {
    serde_json::from_str(text).map_err(|e| SpecError::Parse(format!("{what}: {e}")))
}

impl Spec {
    /// The repo's spec directory: `$MORDHAU_SPEC_DIR`, else `<repo>/data_gen/spec` (this crate is core/crates/mh-spec).
    pub fn default_dir() -> PathBuf {
        if let Ok(d) = std::env::var("MORDHAU_SPEC_DIR") {
            return PathBuf::from(d);
        }
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../data_gen/spec")
    }

    /// Load `index.json`, `fields.json`, `rules.json`, `sources.json` and every `entities/<type>.json` it lists.
    pub fn load(dir: &Path, with_evidence: bool) -> Result<Spec> {
        let read = |rel: &str| -> Result<String> {
            std::fs::read_to_string(dir.join(rel)).map_err(|e| SpecError::Io(format!("{}: {e}", dir.join(rel).display())))
        };
        let index: Index = parse("index.json", &read("index.json")?)?;
        let mut files = vec![
            ("fields.json".to_string(), read("fields.json")?),
            ("rules.json".to_string(), read("rules.json")?),
            ("sources.json".to_string(), read("sources.json")?),
        ];
        if with_evidence {
            files.push(("evidence.json".to_string(), read("evidence.json")?));
        }
        for info in index.entity_types.values() {
            files.push((info.file.clone(), read(&info.file)?));
        }
        Spec::from_parts(index, files)
    }

    /// Build from already-read files (a host with its own I/O): `files` = (path relative to the spec dir, text).
    pub fn from_parts(index: Index, files: Vec<(String, String)>) -> Result<Spec> {
        if index.format != FORMAT {
            return Err(SpecError::Format(format!("format {} != {FORMAT}", index.format)));
        }
        let mut s = Spec {
            index,
            fields: BTreeMap::new(),
            entities: BTreeMap::new(),
            rules: BTreeMap::new(),
            sources: BTreeMap::new(),
            evidence: BTreeMap::new(),
            types: BTreeMap::new(),
        };
        for (name, text) in files {
            match name.as_str() {
                "fields.json" => s.fields = parse(&name, &text)?,
                "rules.json" => s.rules = parse(&name, &text)?,
                "sources.json" => s.sources = parse(&name, &text)?,
                "evidence.json" => s.evidence = parse(&name, &text)?,
                _ => {
                    let et = s
                        .index
                        .entity_types
                        .iter()
                        .find(|(_, i)| i.file == name)
                        .map(|(t, _)| t.clone())
                        .ok_or_else(|| SpecError::Format(format!("{name} is not listed in index.json")))?;
                    let ents: BTreeMap<String, Entity> = parse(&name, &text)?;
                    if ents.len() != s.index.entity_types[&et].entities {
                        return Err(SpecError::Format(format!("{name}: {} entities, index says {}", ents.len(),
                            s.index.entity_types[&et].entities)));
                    }
                    for (id, e) in ents {
                        s.types.insert(id.clone(), et.clone());
                        s.entities.insert(id, e);
                    }
                }
            }
        }
        Ok(s)
    }

    pub fn entity(&self, id: &str) -> Result<&Entity> {
        self.entities.get(id).ok_or_else(|| SpecError::UnknownEntity(id.to_string()))
    }
    pub fn entity_type(&self, id: &str) -> Result<&str> {
        self.types.get(id).map(|s| s.as_str()).ok_or_else(|| SpecError::UnknownEntity(id.to_string()))
    }
    pub fn field(&self, id: &str) -> Result<&Field> {
        self.fields.get(id).ok_or_else(|| SpecError::UnknownField(id.to_string()))
    }
    pub fn rule(&self, id: &str) -> Result<&Rule> {
        self.rules.get(id).ok_or_else(|| SpecError::UnknownRule(id.to_string()))
    }
    /// every entity id of one type ("weapon", "attack", ...), sorted
    pub fn entities_of<'a>(&'a self, entity_type: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.types.iter().filter(move |(_, t)| t.as_str() == entity_type).map(|(i, _)| i.as_str())
    }

    /// the raw value with its declared type checked against `want` (None = any)
    fn raw(&self, entity: &str, field: &str, want: &[FieldType], what: &'static str) -> Result<Option<&Value>> {
        let e = self.entity(entity)?;
        let f = self.field(field)?;
        if f.entity_type != self.types[entity] {
            return Err(SpecError::WrongEntityType(entity.to_string(), field.to_string()));
        }
        if !want.is_empty() && !want.contains(&f.ty) {
            return Err(SpecError::WrongType(field.to_string(), f.ty, what));
        }
        Ok(e.values.get(field))
    }
    fn req(&self, entity: &str, field: &str, want: &[FieldType], what: &'static str) -> Result<&Value> {
        self.raw(entity, field, want, what)?.ok_or_else(|| SpecError::Unset(entity.to_string(), field.to_string()))
    }
    fn bad(entity: &str, field: &str, v: &Value) -> SpecError {
        SpecError::BadValue(entity.to_string(), field.to_string(), v.to_string())
    }

    /// is the field set on this entity (type-checked ids, any value type)
    pub fn has(&self, entity: &str, field: &str) -> Result<bool> {
        Ok(self.raw(entity, field, &[], "any")?.is_some())
    }
    pub fn value(&self, entity: &str, field: &str) -> Result<&Value> {
        self.req(entity, field, &[], "any")
    }
    pub fn f32(&self, entity: &str, field: &str) -> Result<f32> {
        let v = self.req(entity, field, &[FieldType::Float], "f32")?;
        v.as_f64().map(|x| x as f32).ok_or_else(|| Self::bad(entity, field, v))
    }
    pub fn f64(&self, entity: &str, field: &str) -> Result<f64> {
        let v = self.req(entity, field, &[FieldType::Double], "f64")?;
        v.as_f64().ok_or_else(|| Self::bad(entity, field, v))
    }
    pub fn i64(&self, entity: &str, field: &str) -> Result<i64> {
        let v = self.req(entity, field, &[FieldType::Int], "i64")?;
        v.as_i64().ok_or_else(|| Self::bad(entity, field, v))
    }
    pub fn bool(&self, entity: &str, field: &str) -> Result<bool> {
        let v = self.req(entity, field, &[FieldType::Bool], "bool")?;
        v.as_bool().ok_or_else(|| Self::bad(entity, field, v))
    }
    pub fn str(&self, entity: &str, field: &str) -> Result<&str> {
        let v = self.req(entity, field, &[FieldType::String], "str")?;
        v.as_str().ok_or_else(|| Self::bad(entity, field, v))
    }
    /// a reference to another entity (None = no reference, e.g. a native default motion)
    pub fn reference(&self, entity: &str, field: &str) -> Result<Option<&str>> {
        match self.raw(entity, field, &[FieldType::Ref], "ref")? {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_str().map(Some).ok_or_else(|| Self::bad(entity, field, v)),
        }
    }
    fn f32s(&self, entity: &str, field: &str, want: &[FieldType], what: &'static str) -> Result<Vec<f32>> {
        let v = self.req(entity, field, want, what)?;
        let a = v.as_array().ok_or_else(|| Self::bad(entity, field, v))?;
        a.iter().map(|x| x.as_f64().map(|x| x as f32).ok_or_else(|| Self::bad(entity, field, v))).collect()
    }
    pub fn vec2(&self, entity: &str, field: &str) -> Result<[f32; 2]> {
        let a = self.f32s(entity, field, &[FieldType::Vec2], "vec2")?;
        Ok([a[0], a[1]])
    }
    pub fn vec3(&self, entity: &str, field: &str) -> Result<[f32; 3]> {
        let a = self.f32s(entity, field, &[FieldType::Vec3], "vec3")?;
        Ok([a[0], a[1], a[2]])
    }
    pub fn color(&self, entity: &str, field: &str) -> Result<[f32; 4]> {
        let a = self.f32s(entity, field, &[FieldType::Color], "color")?;
        Ok([a[0], a[1], a[2], a[3]])
    }
    pub fn f32_list(&self, entity: &str, field: &str) -> Result<Vec<f32>> {
        self.f32s(entity, field, &[FieldType::FloatList], "f32_list")
    }
    pub fn i64_list(&self, entity: &str, field: &str) -> Result<Vec<i64>> {
        let v = self.req(entity, field, &[FieldType::IntList], "i64_list")?;
        let a = v.as_array().ok_or_else(|| Self::bad(entity, field, v))?;
        a.iter().map(|x| x.as_i64().ok_or_else(|| Self::bad(entity, field, v))).collect()
    }
    pub fn string_list(&self, entity: &str, field: &str) -> Result<Vec<&str>> {
        let v = self.req(entity, field, &[FieldType::StringList], "string_list")?;
        let a = v.as_array().ok_or_else(|| Self::bad(entity, field, v))?;
        a.iter().map(|x| x.as_str().ok_or_else(|| Self::bad(entity, field, v))).collect()
    }
    pub fn json(&self, entity: &str, field: &str) -> Result<&Value> {
        self.req(entity, field, &[FieldType::Json], "json")
    }

    // ---- record views (sheets r8: loaders read a whole record by field name) ----------------------------------

    /// One entity as a record: typed reads by the record's snake_case field name (`rec.f32("max_walk_speed")`), the
    /// ID built from the entity type's code (`FLD_MOV_MAX_WALK_SPEED`). The reads type-check like the ID getters.
    /// The id may be a temporary (`&format!(..)`): the record borrows the spec's own copy of it.
    pub fn record<'a>(&'a self, entity: &str) -> Result<Record<'a>> {
        let (key, _) = self.entities.get_key_value(entity).ok_or_else(|| SpecError::UnknownEntity(entity.to_string()))?;
        let code = self.code(self.entity_type(key)?)?;
        Ok(Record { spec: self, entity: key.as_str(), code })
    }

    /// An entity's values as a JSON object keyed by field name; names that start with one of `groups` + "_"
    /// (`"scoring"` -> `scoring_kill_score_change`) go into a nested object under that group (`{"scoring":
    /// {"kill_score_change": ..}}`), which is the shape of the reference's nested records (MordhauModeData.scoring,
    /// .state, .meta). Floats stay their f32 decimal.
    pub fn nested(&self, entity: &str, groups: &[&str]) -> Result<Value> {
        let e = self.entity(entity)?;
        let mut out = serde_json::Map::new();
        for (fid, v) in &e.values {
            let name = &self.field(fid)?.name;
            match groups.iter().find(|g| name.starts_with(&format!("{g}_"))) {
                Some(g) => {
                    let sub = out.entry(g.to_string()).or_insert_with(|| Value::Object(Default::default()));
                    if let Value::Object(m) = sub {
                        m.insert(name[g.len() + 1..].to_string(), v.clone());
                    }
                }
                None => {
                    out.insert(name.clone(), v.clone());
                }
            }
        }
        Ok(Value::Object(out))
    }

    /// A UCurveFloat (`curve` entity: FRichCurve keys + extrapolation, CombatData.curve_keys / curve_extrap) by its
    /// entity id or its package path.
    pub fn curve(&self, id_or_path: &str) -> Result<Curve> {
        let id = if id_or_path.starts_with("ENT_") {
            id_or_path.to_string()
        } else {
            self.entity_id("curve", id_or_path.rsplit('/').next().unwrap_or(id_or_path))?
        };
        let r = self.record(&id)?;
        let t = r.f32_list("times")?;
        let v = r.f32_list("values")?;
        let m = r.string_list("interp")?;
        Ok(Curve {
            package: r.str("package")?.to_string(),
            keys: t.into_iter().zip(v).zip(m).map(|((t, v), m)| (t, v, m.to_string())).collect(),
            pre: r.str("pre_extrap").unwrap_or("").to_string(),
            post: r.str("post_extrap").unwrap_or("").to_string(),
        })
    }

    /// The mod layer as a spec overlay: `{"<ENT id>": {"<FLD id>": value}}` replaces those values in place (a
    /// server mod's changed class defaults, e.g. mh-parity's --layer). Every id must exist and belong together, and
    /// every value must fit the field's type (a `float` is rounded to its f32, as the exe holds it). Returns how many
    /// values were replaced; on any error nothing is changed.
    pub fn apply_overlay(&mut self, overlay: &Value) -> Result<usize> {
        let o = overlay.as_object().ok_or_else(|| SpecError::Format("overlay: not an object".into()))?;
        let mut todo = Vec::new();
        for (eid, fields) in o {
            let et = self.entity_type(eid)?.to_string();
            for (fid, v) in fields.as_object().ok_or_else(|| SpecError::Format(format!("overlay.{eid}: not an object")))? {
                let f = self.field(fid)?;
                if f.entity_type != et {
                    return Err(SpecError::WrongEntityType(eid.clone(), fid.clone()));
                }
                let ok = match f.ty {
                    FieldType::Bool => v.is_boolean(),
                    FieldType::Int => v.is_i64() || v.is_u64(),
                    FieldType::Float | FieldType::Double => v.is_number(),
                    FieldType::String => v.is_string(),
                    FieldType::Ref => v.is_string() || v.is_null(),
                    FieldType::Vec2 | FieldType::Vec3 | FieldType::Color | FieldType::FloatList | FieldType::IntList => {
                        v.as_array().is_some_and(|a| a.iter().all(|x| x.is_number()))
                    }
                    FieldType::StringList => v.as_array().is_some_and(|a| a.iter().all(|x| x.is_string())),
                    FieldType::Json => true,
                };
                if !ok {
                    return Err(SpecError::BadValue(eid.clone(), fid.clone(), v.to_string()));
                }
                let v = if f.ty == FieldType::Float { serde_json::json!(v.as_f64().unwrap() as f32 as f64) } else { v.clone() };
                todo.push((eid.clone(), fid.clone(), v));
            }
        }
        let n = todo.len();
        for (eid, fid, v) in todo {
            self.entities.get_mut(&eid).unwrap().values.insert(fid, v);
        }
        Ok(n)
    }

    // ---- rules implemented once ------------------------------------------------------------------------------

    /// RULE_ATTACK_SLOT_BY_MOVE (AMordhauWeapon::GetBaseAttackInfo rva=0x1620c50): the FAttackInfo slot ("strike",
    /// "stab", "kick", ...) an EAttackMove (`"RIGHT_STRIKE"`, `"STAB"`, ...; CombatEnums.Move names) reads.
    pub fn attack_slot_for_move(&self, mv: &str) -> Result<&str> {
        let r = self.rule("RULE_ATTACK_SLOT_BY_MOVE")?;
        r.params["attack_slot_by_move"][mv]
            .as_str()
            .ok_or_else(|| SpecError::BadValue("RULE_ATTACK_SLOT_BY_MOVE".into(), mv.into(), "no slot for this move".into()))
    }

    /// RULE_ALT_MODE_ATTACK_SWAP (AMordhauWeapon::OnRequestModeSwitch_Implementation rva=0x16327a0): the field a
    /// weapon reads in alternate mode instead of `name` (the swap pairs are symmetric; unpaired names map to themselves)
    pub fn alt_mode_field<'a>(&'a self, name: &'a str) -> Result<&'a str> {
        let r = self.rule("RULE_ALT_MODE_ATTACK_SWAP")?;
        for p in r.params["alt_mode_swaps"].as_array().into_iter().flatten() {
            let (a, b) = (p[0].as_str().unwrap_or(""), p[1].as_str().unwrap_or(""));
            if a == name {
                return Ok(b);
            }
            if b == name {
                return Ok(a);
            }
        }
        Ok(name)
    }

    /// The attack entity (`ENT_ATK_*`) a weapon's EAttackMove uses, in primary or alternate mode: the weapon's
    /// `attack_<slot>` reference after both rules above. None = the weapon has no such attack.
    pub fn attack_for_move(&self, weapon: &str, mv: &str, alternate: bool) -> Result<Option<&str>> {
        let mut slot = self.attack_slot_for_move(mv)?;
        if alternate {
            slot = self.alt_mode_field(slot)?;
        }
        let code = self.code(self.entity_type(weapon)?)?;
        self.reference(weapon, &ids::field(code, &format!("attack_{slot}")))
    }

    /// the ID code of an entity type, from index.json ("weapon" -> "WPN")
    pub fn code(&self, entity_type: &str) -> Result<&str> {
        self.index
            .entity_types
            .get(entity_type)
            .map(|i| i.code.as_str())
            .ok_or_else(|| SpecError::Format(format!("no entity type {entity_type}")))
    }
    /// ENT_<code>_<key> of an entity type
    pub fn entity_id(&self, entity_type: &str, key: &str) -> Result<String> {
        Ok(ids::entity(self.code(entity_type)?, key))
    }
    /// FLD_<code>_<NAME> of an entity type
    pub fn field_id(&self, entity_type: &str, name: &str) -> Result<String> {
        Ok(ids::field(self.code(entity_type)?, name))
    }
}

/// A record view of one entity (`Spec::record`): the ID getters by snake_case field name.
#[derive(Clone, Copy)]
pub struct Record<'a> {
    pub spec: &'a Spec,
    pub entity: &'a str,
    pub code: &'a str,
}

impl<'a> Record<'a> {
    fn fid(&self, name: &str) -> String {
        ids::field(self.code, name)
    }
    pub fn has(&self, name: &str) -> bool {
        self.spec.has(self.entity, &self.fid(name)).unwrap_or(false)
    }
    pub fn f32(&self, name: &str) -> Result<f32> {
        self.spec.f32(self.entity, &self.fid(name))
    }
    pub fn f64(&self, name: &str) -> Result<f64> {
        self.spec.f64(self.entity, &self.fid(name))
    }
    pub fn i64(&self, name: &str) -> Result<i64> {
        self.spec.i64(self.entity, &self.fid(name))
    }
    pub fn bool(&self, name: &str) -> Result<bool> {
        self.spec.bool(self.entity, &self.fid(name))
    }
    pub fn str(&self, name: &str) -> Result<&'a str> {
        let fid = self.fid(name);
        let s: &'a Spec = self.spec;
        let v = s.req(self.entity, &fid, &[FieldType::String], "str")?;
        v.as_str().ok_or_else(|| Spec::bad(self.entity, &fid, v))
    }
    pub fn reference(&self, name: &str) -> Result<Option<&'a str>> {
        let fid = self.fid(name);
        let s: &'a Spec = self.spec;
        match s.raw(self.entity, &fid, &[FieldType::Ref], "ref")? {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v.as_str().map(Some).ok_or_else(|| Spec::bad(self.entity, &fid, v)),
        }
    }
    pub fn vec2(&self, name: &str) -> Result<[f32; 2]> {
        self.spec.vec2(self.entity, &self.fid(name))
    }
    pub fn vec3(&self, name: &str) -> Result<[f32; 3]> {
        self.spec.vec3(self.entity, &self.fid(name))
    }
    pub fn f32_list(&self, name: &str) -> Result<Vec<f32>> {
        self.spec.f32_list(self.entity, &self.fid(name))
    }
    pub fn string_list(&self, name: &str) -> Result<Vec<&'a str>> {
        let fid = self.fid(name);
        let s: &'a Spec = self.spec;
        let v = s.req(self.entity, &fid, &[FieldType::StringList], "string_list")?;
        let a = v.as_array().ok_or_else(|| Spec::bad(self.entity, &fid, v))?;
        a.iter().map(|x| x.as_str().ok_or_else(|| Spec::bad(self.entity, &fid, v))).collect()
    }
    pub fn json(&self, name: &str) -> Result<&'a Value> {
        let fid = self.fid(name);
        let s: &'a Spec = self.spec;
        s.req(self.entity, &fid, &[FieldType::Json], "json")
    }
    /// the raw value by name, any type (None = not set on this entity)
    pub fn value(&self, name: &str) -> Option<&'a Value> {
        let s: &'a Spec = self.spec;
        s.entities.get(self.entity).and_then(|e| e.values.get(&ids::field(self.code, name)))
    }
}

/// A UCurveFloat's FRichCurve from the spec (`Spec::curve`): keys (time, value, interp mode) and extrapolation.
#[derive(Clone, Debug, PartialEq)]
pub struct Curve {
    pub package: String,
    pub keys: Vec<(f32, f32, String)>,
    pub pre: String,
    pub post: String,
}

/// ID builders over an ID code (docs/SPEC_SHEETS.md "IDs"; `Spec::code` maps an entity type to its code):
/// characters outside [A-Za-z0-9_] become '_'.
pub mod ids {
    fn clean(s: &str) -> String {
        s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' }).collect()
    }
    /// ENT_<CODE>_<key>, e.g. entity("WPN", "BP_Longsword") = "ENT_WPN_BP_Longsword"
    pub fn entity(code: &str, key: &str) -> String {
        format!("ENT_{code}_{}", clean(key))
    }
    /// FLD_<CODE>_<NAME>, e.g. field("ATK", "windup") = "FLD_ATK_WINDUP"
    pub fn field(code: &str, name: &str) -> String {
        format!("FLD_{code}_{}", clean(name).to_ascii_uppercase())
    }
}

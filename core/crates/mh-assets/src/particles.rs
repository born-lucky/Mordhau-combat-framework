//! Cascade particle systems (UParticleSystem; Mordhau's effects are Cascade: the paks hold no NiagaraSystem, only 58
//! NiagaraScript / data-interface assets unused by the gameplay effects) as engine-neutral data, plus a CPU reference
//! of their distributions. Read from the paks (mh-pak export JSON):
//!   UParticleSystem.Emitters[] -> UParticleSpriteEmitter.LODLevels[] -> UParticleLODLevel {RequiredModule,
//!   SpawnModule, TypeDataModule, Modules[]} -> UParticleModule* (every module's tagged properties are kept; each
//!   property that is a FRawDistributionFloat / FRawDistributionVector is decoded into a `Dist`).
//! Cooked distributions carry a baked FDistributionLookupTable (Distribution = null) and the runtime evaluates that
//! table, read from the shipped exe (scripts/ue_dis.py):
//!   FRawDistributionFloat::GetValue rva=0x308b4f0 (table layout: TimeScale +0, TimeBias +4, Values +8, Op +0x18,
//!   EntryCount +0x19, EntryStride +0x1a, SubEntryStride +0x1b, LockFlag +0x1c): with a table (Values non-empty,
//!   EntryCount != 0): t' = max((t - TimeBias) * TimeScale, 0); i = trunc(t'), alpha = t' - floor(t'); entries
//!   e0 = min(i, EntryCount - 1) * EntryStride, e1 = min(i + 1, EntryCount - 1) * EntryStride;
//!   Op 1 (RDO_None, helper 0x308afd0): lerp(V[e0], V[e1], alpha);
//!   Op 2 (RDO_Random): u = rand; lerp(lerp(V[e0], V[e1], a), lerp(V[e0 + 1], V[e1 + 1], a), u);
//!   Op 3 (RDO_Extreme): k = u > 0.5 ? 1 : 0; lerp(V[e0 + k], V[e1 + k], a); other Op -> 0; no table -> the
//!   Distribution object (DistributionFloatConstant etc.).
//!   FRawDistributionVector::GetValue rva=0x308b750: Op 1 helper 0x308b1d0 (lerp of 3 components), Op 2 helper
//!   0x308b2b0: three draws (x, y, z in that order), LockFlag 1 (XY) y = x, 2 (XZ) z = x, 3 (YZ) z = y, 4 (XYZ)
//!   y = z = x; lo = V[e + 0..3], hi = V[e + 3..6] (sub-entry stride 3 hard-coded); Op 3 helper 0x308b070.
//!   rand = FRandomStream::GetFraction when a stream is given: seed = seed * 196314165 + 907633515, value =
//!   float((seed >> 9) | 0x3f800000) - 1 (else FMath::FRand); `RandomStream` ports it.
//! Distribution objects (DistributionFloatConstant {Constant}, ...Uniform {Min, Max}, ...ConstantCurve {ConstantCurve
//! Points}; Vector variants): UE 4.26 Distributions (UNCONFIRMED: recalled, not read from the exe).

use crate::material::strip_index;
use mh_pak::Reader;
use serde_json::Value;
use std::collections::BTreeMap;

/// FRandomStream (Math/RandomStream.h; the constants are in the GetValue code above)
#[derive(Debug, Clone, Copy)]
pub struct RandomStream(pub u32);

impl RandomStream {
    pub fn fraction(&mut self) -> f32 {
        self.0 = self.0.wrapping_mul(196_314_165).wrapping_add(907_633_515);
        f32::from_bits((self.0 >> 9) | 0x3f80_0000) - 1.0
    }
}

/// FDistributionLookupTable
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LookupTable {
    pub time_scale: f32,
    pub time_bias: f32,
    pub values: Vec<f32>,
    /// ERawDistributionOperation: 1 None, 2 Random, 3 Extreme
    pub op: u8,
    pub entry_count: u8,
    pub entry_stride: u8,
    pub sub_entry_stride: u8,
    pub lock_flag: u8,
}

/// FInterpCurvePoint of a curve distribution
#[derive(Debug, Clone, PartialEq)]
pub struct CurvePoint {
    pub in_val: f32,
    pub out: [f32; 3],
    pub interp: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DistObject {
    Constant([f32; 3]),
    Uniform([f32; 3], [f32; 3]),
    Curve(Vec<CurvePoint>),
    /// a distribution class not decoded (its properties kept in the module's raw JSON)
    Other(String),
}

/// A FRawDistributionFloat / FRawDistributionVector property
#[derive(Debug, Clone, PartialEq)]
pub struct Dist {
    pub table: Option<LookupTable>,
    pub object: Option<DistObject>,
    /// MinValue / MaxValue (float) and MinValueVec / MaxValueVec as cooked
    pub min: f32,
    pub max: f32,
    pub min_vec: [f32; 3],
    pub max_vec: [f32; 3],
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

impl LookupTable {
    fn entries(&self, t: f32) -> (usize, usize, f32) {
        let x = ((t - self.time_bias) * self.time_scale).max(0.0);
        let i = x as i64 as usize;
        let last = (self.entry_count as usize).saturating_sub(1);
        let s = self.entry_stride as usize;
        (i.min(last) * s, (i + 1).min(last) * s, x - x.floor())
    }
    fn v(&self, i: usize) -> f32 {
        self.values.get(i).copied().unwrap_or(0.0)
    }
    /// FRawDistributionFloat::GetValue with a table
    pub fn value1(&self, t: f32, rand: &mut dyn FnMut() -> f32) -> f32 {
        let (e0, e1, a) = self.entries(t);
        match self.op {
            1 => lerp(self.v(e0), self.v(e1), a),
            2 => {
                let u = rand();
                let lo = lerp(self.v(e0), self.v(e1), a);
                let hi = lerp(self.v(e0 + 1), self.v(e1 + 1), a);
                lo + (hi - lo) * u
            }
            3 => {
                let k = if rand() > 0.5 { 1 } else { 0 };
                lerp(self.v(e0 + k), self.v(e1 + k), a)
            }
            _ => 0.0,
        }
    }
    /// FRawDistributionVector::GetValue with a table
    pub fn value3(&self, t: f32, rand: &mut dyn FnMut() -> f32) -> [f32; 3] {
        let (e0, e1, a) = self.entries(t);
        let at = |o: usize| [0, 1, 2].map(|c| lerp(self.v(e0 + o + c), self.v(e1 + o + c), a));
        match self.op {
            1 => at(0),
            2 => {
                let mut u = [rand(), rand(), rand()];
                match self.lock_flag {
                    1 => u[1] = u[0],
                    2 => u[2] = u[0],
                    3 => u[2] = u[1],
                    4 => {
                        u[1] = u[0];
                        u[2] = u[0];
                    }
                    _ => {}
                }
                let (lo, hi) = (at(0), at(3));
                [0, 1, 2].map(|c| lo[c] + (hi[c] - lo[c]) * u[c])
            }
            // UNCONFIRMED: helper 0x308b070 not read; one draw for the vector, as the float path
            3 => at(if rand() > 0.5 { 3 } else { 0 }),
            _ => [0.0; 3],
        }
    }
}

impl Dist {
    /// GetValue for a float distribution at time t (normalized particle / emitter time, as the module passes it)
    pub fn value1(&self, t: f32, rand: &mut dyn FnMut() -> f32) -> f32 {
        if let Some(tb) = self.table.as_ref().filter(|tb| !tb.values.is_empty() && tb.entry_count != 0) {
            return tb.value1(t, rand);
        }
        match &self.object {
            Some(DistObject::Constant(c)) => c[0],
            Some(DistObject::Uniform(a, b)) => lerp(a[0], b[0], rand()),
            Some(DistObject::Curve(p)) => curve_eval(p, t)[0],
            _ => 0.0,
        }
    }
    pub fn value3(&self, t: f32, rand: &mut dyn FnMut() -> f32) -> [f32; 3] {
        if let Some(tb) = self.table.as_ref().filter(|tb| !tb.values.is_empty() && tb.entry_count != 0) {
            return tb.value3(t, rand);
        }
        match &self.object {
            Some(DistObject::Constant(c)) => *c,
            Some(DistObject::Uniform(a, b)) => {
                let u = [rand(), rand(), rand()];
                [0, 1, 2].map(|i| lerp(a[i], b[i], u[i]))
            }
            Some(DistObject::Curve(p)) => curve_eval(p, t),
            _ => [0.0; 3],
        }
    }
}

/// FInterpCurve eval, linear / constant between points (UNCONFIRMED: cubic points evaluated linearly)
fn curve_eval(p: &[CurvePoint], t: f32) -> [f32; 3] {
    if p.is_empty() {
        return [0.0; 3];
    }
    if t <= p[0].in_val {
        return p[0].out;
    }
    for w in p.windows(2) {
        if t <= w[1].in_val {
            if w[0].interp.ends_with("Constant") {
                return w[0].out;
            }
            let a = (t - w[0].in_val) / (w[1].in_val - w[0].in_val).max(1e-12);
            return [0, 1, 2].map(|c| lerp(w[0].out[c], w[1].out[c], a));
        }
    }
    p[p.len() - 1].out
}

#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    pub class: String,
    pub name: String,
    /// the module's tagged properties as stored
    pub props: Value,
    /// every FRawDistribution property, by property name
    pub dists: BTreeMap<String, Dist>,
}

impl Module {
    pub fn dist(&self, k: &str) -> Option<&Dist> {
        self.dists.get(k)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Lod {
    pub required: Option<Module>,
    pub spawn: Option<Module>,
    pub type_data: Option<Module>,
    pub modules: Vec<Module>,
}

impl Lod {
    /// UParticleModuleRequired Material package
    pub fn material(&self) -> String {
        self.required.as_ref().and_then(|r| r.props.pointer("/Material/ObjectPath")).and_then(Value::as_str).map(|s| strip_index(s).to_string()).unwrap_or_default()
    }
    /// UParticleModuleSpawn BurstList [(Count, CountLow, Time)] (CountLow -1 = Count)
    pub fn bursts(&self) -> Vec<(i64, i64, f64)> {
        self.spawn
            .as_ref()
            .and_then(|s| s.props.get("BurstList"))
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .map(|b| (b.get("Count").and_then(Value::as_i64).unwrap_or(0), b.get("CountLow").and_then(Value::as_i64).unwrap_or(-1), b.get("Time").and_then(Value::as_f64).unwrap_or(0.0)))
                    .collect()
            })
            .unwrap_or_default()
    }
    pub fn module(&self, class: &str) -> Option<&Module> {
        self.modules.iter().find(|m| m.class == class)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Emitter {
    pub name: String,
    pub class: String,
    pub props: Value,
    pub lods: Vec<Lod>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ParticleSystem {
    pub package: String,
    pub props: Value,
    pub emitters: Vec<Emitter>,
    pub lod_distances: Vec<f64>,
}

fn idx(v: Option<&Value>) -> Option<usize> {
    v?.get("ObjectPath")?.as_str()?.rsplit_once('.')?.1.parse().ok()
}
fn f32v(v: &Value, k: &str, d: f32) -> f32 {
    v.get(k).and_then(Value::as_f64).map_or(d, |x| x as f32)
}
fn vec3(v: Option<&Value>, d: f32) -> [f32; 3] {
    match v {
        Some(v) if v.is_object() => [f32v(v, "X", d), f32v(v, "Y", d), f32v(v, "Z", d)],
        _ => [d; 3],
    }
}

fn dist_object(ex: &[Value], v: Option<&Value>) -> Option<DistObject> {
    let e = ex.get(idx(v)?)?;
    let t = e.get("Type").and_then(Value::as_str).unwrap_or("");
    let null = Value::Null;
    let p = e.get("Properties").unwrap_or(&null);
    Some(match t {
        "DistributionFloatConstant" => DistObject::Constant([f32v(p, "Constant", 0.0), 0.0, 0.0]),
        "DistributionVectorConstant" => DistObject::Constant(vec3(p.get("Constant"), 0.0)),
        "DistributionFloatUniform" => DistObject::Uniform([f32v(p, "Min", 0.0), 0.0, 0.0], [f32v(p, "Max", 0.0), 0.0, 0.0]),
        "DistributionVectorUniform" => DistObject::Uniform(vec3(p.get("Min"), 0.0), vec3(p.get("Max"), 0.0)),
        "DistributionFloatConstantCurve" | "DistributionVectorConstantCurve" => DistObject::Curve(
            p.pointer("/ConstantCurve/Points")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .map(|q| CurvePoint {
                            in_val: f32v(q, "InVal", 0.0),
                            out: match q.get("OutVal") {
                                Some(o) if o.is_object() => vec3(Some(o), 0.0),
                                Some(o) => [o.as_f64().unwrap_or(0.0) as f32, 0.0, 0.0],
                                None => [0.0; 3],
                            },
                            interp: q.get("InterpMode").and_then(Value::as_str).unwrap_or("CIM_Linear").to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default(),
        ),
        other => DistObject::Other(other.to_string()),
    })
}

fn is_dist(v: &Value) -> bool {
    v.is_object() && (v.get("Table").is_some() || v.get("Distribution").is_some())
}

fn dist(ex: &[Value], v: &Value) -> Dist {
    let table = v.get("Table").filter(|t| t.is_object()).map(|t| LookupTable {
        time_scale: f32v(t, "TimeScale", 0.0),
        time_bias: f32v(t, "TimeBias", 0.0),
        values: t.get("Values").and_then(Value::as_array).map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default(),
        op: t.get("Op").and_then(Value::as_u64).unwrap_or(0) as u8,
        entry_count: t.get("EntryCount").and_then(Value::as_u64).unwrap_or(0) as u8,
        entry_stride: t.get("EntryStride").and_then(Value::as_u64).unwrap_or(0) as u8,
        sub_entry_stride: t.get("SubEntryStride").and_then(Value::as_u64).unwrap_or(0) as u8,
        lock_flag: t.get("LockFlag").and_then(Value::as_u64).unwrap_or(0) as u8,
    });
    Dist {
        table,
        object: dist_object(ex, v.get("Distribution")),
        min: f32v(v, "MinValue", 0.0),
        max: f32v(v, "MaxValue", 0.0),
        min_vec: vec3(v.get("MinValueVec"), 0.0),
        max_vec: vec3(v.get("MaxValueVec"), 0.0),
    }
}

fn module(ex: &[Value], r: Option<&Value>) -> Option<Module> {
    let e = ex.get(idx(r)?)?;
    let p = e.get("Properties").cloned().unwrap_or(Value::Null);
    let mut dists = BTreeMap::new();
    if let Some(o) = p.as_object() {
        for (k, v) in o {
            if is_dist(v) {
                dists.insert(k.clone(), dist(ex, v));
            }
            // arrays of structs carrying distributions (ParameterDynamic DynamicParams[i].ParamValue, ...)
            for (i, e) in v.as_array().into_iter().flatten().enumerate() {
                for (kk, vv) in e.as_object().into_iter().flatten() {
                    if is_dist(vv) {
                        dists.insert(format!("{k}[{i}].{kk}"), dist(ex, vv));
                    }
                }
            }
        }
    }
    Some(Module {
        class: e.get("Type").and_then(Value::as_str).unwrap_or("").to_string(),
        name: e.get("Name").and_then(Value::as_str).unwrap_or("").to_string(),
        props: p,
        dists,
    })
}

/// The ParticleSystem of a package
pub fn read(rd: &Reader, pkg: &str) -> Option<ParticleSystem> {
    let pkg = strip_index(pkg);
    let ex = rd.read(pkg)?;
    let ps = ex.iter().find(|e| e.get("Type").and_then(Value::as_str) == Some("ParticleSystem"))?;
    let null = Value::Null;
    let pp = ps.get("Properties").unwrap_or(&null);
    let mut emitters = Vec::new();
    for er in pp.get("Emitters").and_then(Value::as_array).into_iter().flatten() {
        let Some(ee) = idx(Some(er)).and_then(|i| ex.get(i)) else { continue };
        let ep = ee.get("Properties").cloned().unwrap_or(Value::Null);
        let mut lods = Vec::new();
        for lr in ep.get("LODLevels").and_then(Value::as_array).into_iter().flatten() {
            let Some(le) = idx(Some(lr)).and_then(|i| ex.get(i)) else { continue };
            let lp = le.get("Properties").unwrap_or(&null);
            lods.push(Lod {
                required: module(&ex, lp.get("RequiredModule")),
                spawn: module(&ex, lp.get("SpawnModule")),
                type_data: module(&ex, lp.get("TypeDataModule")),
                modules: lp.get("Modules").and_then(Value::as_array).into_iter().flatten().filter_map(|m| module(&ex, Some(m))).collect(),
            });
        }
        emitters.push(Emitter {
            name: ee.get("Name").and_then(Value::as_str).unwrap_or("").to_string(),
            class: ee.get("Type").and_then(Value::as_str).unwrap_or("").to_string(),
            props: ep,
            lods,
        });
    }
    Some(ParticleSystem {
        package: pkg.to_string(),
        lod_distances: pp.get("LODDistances").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_f64).collect()).unwrap_or_default(),
        props: pp.clone(),
        emitters,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tb(op: u8, n: u8, stride: u8, values: &[f32]) -> LookupTable {
        LookupTable { time_scale: (n as f32 - 1.0).max(1.0), time_bias: 0.0, values: values.to_vec(), op, entry_count: n, entry_stride: stride, sub_entry_stride: 0, lock_flag: 0 }
    }

    #[test]
    fn table_ops_as_the_exe() {
        // Op 1 two entries over t in [0, 1]: lerp
        let t = tb(1, 2, 1, &[1.0, 0.0]);
        assert_eq!(t.value1(0.25, &mut || 0.0), 0.75);
        assert_eq!(t.value1(5.0, &mut || 0.0), 0.0); // past the end: both entries = last
        // Op 2 single entry [0.5, 1.5]: lerp(lo, hi, u)
        let r = tb(2, 1, 2, &[0.5, 1.5]);
        assert_eq!(r.value1(0.3, &mut || 0.25), 0.75);
        // Op 3 extreme picks the high value above 0.5
        let x = tb(3, 1, 2, &[2.0, 7.0]);
        assert_eq!((x.value1(0.0, &mut || 0.4), x.value1(0.0, &mut || 0.6)), (2.0, 7.0));
        // vector random with XYZ lock: one draw for all
        let mut v = tb(2, 1, 6, &[-1.0, -1.0, -1.0, 1.0, 1.0, 1.0]);
        v.lock_flag = 4;
        let mut k = 0;
        let mut seq = || {
            k += 1;
            [0.75f32, 0.0, 0.0][k - 1]
        };
        assert_eq!(v.value3(0.0, &mut seq), [0.5, 0.5, 0.5]);
    }

    #[test]
    fn random_stream_matches_ue_constants() {
        let mut s = RandomStream(0);
        let a = s.fraction();
        assert_eq!(s.0, 907_633_515);
        assert!((0.0..1.0).contains(&a));
    }
}

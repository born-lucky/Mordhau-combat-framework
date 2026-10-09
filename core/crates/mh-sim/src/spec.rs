//! The combat records from the spec matrix (mh-spec, data_gen/spec: every UE float stored as its f32) plus what the
//! matrix does not carry, read from the game's own paks (mh-pak) and the PDB / .rdata dumps:
//!   - UCurveFloat keys (the motions' curve fields; CombatData.curve_keys / curve_extrap),
//!   - the weapon trace sockets TraceStart / TraceEnd (UePhysics.weapon_mesh / socket),
//!   - the character physics asset bodies (physics.rs),
//!   - BP_MordhauCharacter's BlockCollider template (CombatData.block_collider) and CharacterMesh0 placement,
//!   - native class chains (PDB class records, extract/native/types/<cls>.h `class X : public Y`),
//!   - the bone FNames and the FRichCurve 1/3 (.rdata via extract/native/rdata.tsv, as CombatConstants reads them).
//! Output: the RecordsJson tree (mordhau-core data::load_records), so the combat core reads the same `Spec` either way.

use mh_pak::Reader;
use mordhau_core::data::{load_records, round_f32, Spec};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub type Result<T> = std::result::Result<T, String>;

/// the repo root (MORDHAU_REPO overrides the build-time location, e.g. for an out-of-tree build)
fn repo() -> PathBuf {
    match std::env::var("MORDHAU_REPO") {
        Ok(d) => PathBuf::from(d),
        Err(_) => Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."),
    }
}

/// EAttackMove of the matrix's move-keyed fields (motion_<move>, alt_motion_<move>)
const MOVES: [(&str, i64); 6] = [("right_strike", 0), ("left_strike", 1), ("stab", 2), ("alt_stab", 3), ("kick", 4), ("couch", 6)];

pub struct SpecBuilder<'a> {
    pub m: &'a mh_spec::Spec,
    pub rd: &'a Reader,
    /// extract/native (PDB types, rdata.tsv); the PDB dump the reference's NativeCtor / UeRdata read
    pub native_dir: PathBuf,
}

/// field values of an entity with the type prefix dropped: FLD_WPN_B_CAN_BLOCK -> b_can_block
fn plain(e: &mh_spec::Entity) -> Map<String, Value> {
    let mut o = Map::new();
    for (k, v) in &e.values {
        let name = k.splitn(3, '_').nth(2).unwrap_or(k).to_lowercase();
        o.insert(name, v.clone());
    }
    o
}

fn base_name(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

impl<'a> SpecBuilder<'a> {
    pub fn new(m: &'a mh_spec::Spec, rd: &'a Reader) -> Self {
        SpecBuilder { m, rd, native_dir: repo().join("extract/native") }
    }

    fn ent(&self, id: &str) -> Result<Map<String, Value>> {
        self.m.entities.get(id).map(plain).ok_or_else(|| format!("spec: no entity {id}"))
    }

    fn weapon_id(path: &str) -> String {
        format!("ENT_WPN_{}", base_name(path))
    }

    /// motion entity id -> its key in data::Spec (Blueprint path, or native:<cls>)
    fn motion_key(&self, id: &str) -> Result<String> {
        if let Some(cls) = id.strip_prefix("ENT_MOT_NATIVE_") {
            return Ok(format!("native:{cls}"));
        }
        let e = self.ent(id)?;
        Ok(e.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string())
    }

    /// The whole Spec for these weapons (left hands included), the kick weapon and the fists, and every game mode.
    pub fn build(&self, weapon_paths: &[&str]) -> Result<Spec> {
        let mut root = Map::new();
        let mut motion_ids: BTreeSet<String> = ["ENT_MOT_NATIVE_UAttackMotion", "ENT_MOT_NATIVE_UIdleMotion", "ENT_MOT_NATIVE_UParryMotion"].iter().map(|s| s.to_string()).collect();
        for n in ["BP_FeintedMotion", "BP_BlockedMotion", "BP_FlinchMotion", "BP_StunMotion", "BP_DisarmedMotion"] {
            motion_ids.insert(format!("ENT_MOT_{n}"));
        }
        // character + stats + kick weapon
        let mut ch = self.ent("ENT_CHR_BP_MordhauCharacter")?;
        if !ch.contains_key("block_collider_forward_parry_distance") {
            // not in the matrix yet: the BP_MordhauCharacter CDO chain's BlockColliderForwardParryDistance
            // (AMordhauCharacter +0xf54; BP (50, 10) over the native ctor's (25, 10), AMordhauCharacter.cpp 3015-3016)
            let d = self.rd.defaults(crate::physics::CHARACTER);
            if let Some(v) = d.get("BlockColliderForwardParryDistance") {
                ch.insert("block_collider_forward_parry_distance".into(), json!([v["X"].as_f64().unwrap_or(0.0), v["Y"].as_f64().unwrap_or(0.0)]));
            }
        }
        let kick_id = ch.get("kick_weapon").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let kick_path = if kick_id.is_empty() { String::new() } else { self.ent(&kick_id)?.get("class_path").and_then(|v| v.as_str()).unwrap_or("").to_string() };
        root.insert("character".into(), Value::Object(ch));
        root.insert("kick_weapon_path".into(), json!(kick_path));
        let mut stats = Map::new();
        for (cls, id) in [("UStaminaStatComponent", "ENT_STAT_STAMINA"), ("UHealthStatComponent", "ENT_STAT_HEALTH")] {
            stats.insert(cls.into(), Value::Object(self.ent(id)?));
        }
        root.insert("stats".into(), Value::Object(stats));
        // weapons
        let mut paths: Vec<String> = weapon_paths.iter().map(|s| s.to_string()).collect();
        if !kick_path.is_empty() {
            paths.push(kick_path.clone());
        }
        let mut weapons = Map::new();
        let mut profiles = Map::new();
        let mut natives: BTreeSet<String> = BTreeSet::new();
        for p in &paths {
            if weapons.contains_key(p) {
                continue;
            }
            let wid = Self::weapon_id(p);
            let w = self.ent(&wid)?;
            let mut wd = Map::new();
            let mut eq = Map::new();
            for (k, v) in &w {
                if let Some(a) = k.strip_prefix("attack_") {
                    if let Some(aid) = v.as_str() {
                        if a != "mask" && a != "speed_modifier" && a != "supersprint_duration" {
                            wd.insert(a.to_string(), Value::Object(self.ent(aid)?));
                            continue;
                        }
                    }
                }
                if let Some(e) = k.strip_prefix("equip_") {
                    eq.insert(e.to_string(), v.clone());
                    continue;
                }
                wd.insert(k.clone(), v.clone());
            }
            let native = format!("A{}", w.get("native_class").and_then(|v| v.as_str()).unwrap_or(""));
            natives.insert(native.clone());
            eq.insert("path".into(), json!(p));
            eq.insert("native".into(), json!(native));
            for k in ["b_has_alternate_mode", "b_second_is_two_handed", "b_is_two_handed"] {
                if let Some(v) = w.get(k) {
                    eq.insert(k.into(), v.clone());
                }
            }
            // profile -> motions (primary and alternate grip)
            let mut motions = Map::new();
            for (pre, prof_field, parry_field) in [("motion_", "profile", "parry_motion"), ("alt_motion_", "alt_profile", "alt_parry_motion")] {
                let mut pm = Map::new();
                for (mv, n) in MOVES {
                    if let Some(id) = w.get(&format!("{pre}{mv}")).and_then(|v| v.as_str()) {
                        motion_ids.insert(id.to_string());
                        pm.insert(n.to_string(), json!(self.motion_key(id)?));
                    }
                }
                let parry = match w.get(parry_field).and_then(|v| v.as_str()) {
                    Some(id) if !id.is_empty() => {
                        motion_ids.insert(id.to_string());
                        self.motion_key(id)?
                    }
                    _ => String::new(),
                };
                if pre == "motion_" {
                    motions = pm.clone();
                }
                if let Some(prof) = w.get(prof_field).and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                    profiles.insert(prof.to_string(), json!({"motions": pm, "parry_motion": parry}));
                }
            }
            let parry_motion = profiles
                .get(eq.get("weapon_animation_profile").and_then(|v| v.as_str()).unwrap_or(""))
                .map(|v| v["parry_motion"].clone())
                .unwrap_or(json!(""));
            let tracer = self.tracer_sockets(p)?;
            weapons.insert(
                p.clone(),
                json!({"weapon": wd, "equip": eq, "profile_motions": {"motions": motions, "parry_motion": parry_motion}, "tracer": tracer}),
            );
        }
        root.insert("weapons".into(), Value::Object(weapons));
        root.insert("profiles".into(), Value::Object(profiles));
        // motion defs + their curves
        let mut defs = Map::new();
        let mut curves: BTreeSet<String> = BTreeSet::new();
        for id in &motion_ids {
            let mut d = self.ent(id)?;
            let rec = self.m.entities[id].record.trim_start_matches(':').to_string();
            // FHighMidLowSpineSpaceAdditive fields: the matrix keeps them flat (angling_windup_high ...), the records
            // as {high, mid, low}
            for base in ["angling_windup", "angling_release", "riposte_angling_windup", "riposte_angling_release", "angle_additive_left", "angle_additive_right"] {
                let mut hml = Map::new();
                for lvl in ["high", "mid", "low"] {
                    if let Some(v) = d.remove(&format!("{base}_{lvl}")) {
                        hml.insert(lvl.to_string(), v);
                    }
                }
                if !hml.is_empty() {
                    d.insert(base.to_string(), Value::Object(hml));
                }
            }
            d.insert("rec".into(), json!(rec));
            // BackpedalSpeedFactor (+0x68) / ShieldWallSpeedFactor: the motion Blueprint's CDO over the ctor's 1.0
            // (the matrix does not carry them)
            if let Some(p) = d.get("path").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(|s| s.to_string()) {
                let cdo = self.rd.defaults(&p);
                for (k, f) in [("backpedal_speed_factor", "BackpedalSpeedFactor"), ("shield_wall_speed_factor", "ShieldWallSpeedFactor")] {
                    if !d.contains_key(k) {
                        if let Some(v) = cdo.get(f).and_then(|v| v.as_f64()) {
                            d.insert(k.into(), json!(v));
                        }
                    }
                }
                // camera1p piece 5 (2026-10-07): the FPerspective curve fields' FirstPerson halves (FC_ComboBlendCurve1p,
                // FC_ComboWindUpCurve1p, ...) are what animgraph.rs attack_def_1p hands the montages in first person; they
                // have to be in the curve table too, or the custom-curve blend option (alpha_blend 14) falls back to linear
                // (found with the live stab record: the 1P combo blend-in ran linear instead of FC_ComboBlendCurve1p)
                for k in ["BlendInCurve", "ComboBlendInCurve", "MorphBlendInCurve", "RiposteBlendInCurve", "WindUpCurve", "ComboWindUpCurve", "AutoBlendInWeaponCurve", "AutoBlendInSpineCurve"] {
                    if let Some(v) = cdo.get(k).and_then(|v| v.get("FirstPerson")).and_then(|v| v.get("ObjectPath")).and_then(|v| v.as_str()) {
                        let c = crate::physics::strip(v);
                        if !c.is_empty() {
                            curves.insert(c);
                        }
                    }
                }
            }
            if let Some(n) = d.get("native").and_then(|v| v.as_str()) {
                natives.insert(n.to_string());
            }
            for (k, v) in &d {
                if k.ends_with("_curve") {
                    if let Some(c) = v.as_str().filter(|s| !s.is_empty()) {
                        curves.insert(c.to_string());
                    }
                }
            }
            defs.insert(self.motion_key(id)?, Value::Object(d));
        }
        root.insert("motion_defs".into(), Value::Object(defs));
        let mut cj = Map::new();
        for c in curves {
            cj.insert(c.clone(), self.curve(&c)?);
        }
        root.insert("curves".into(), Value::Object(cj));
        // class chains
        let mut chains = Map::new();
        for n in natives {
            chains.insert(n.clone(), json!(self.class_chain(&n)));
        }
        root.insert("class_chains".into(), Value::Object(chains));
        root.insert("constants".into(), self.constants()?);
        root.insert("block_collider".into(), self.block_collider()?);
        // mode rules: "<game_mode>|<game_state>"
        let mut rules = Map::new();
        for (id, e) in &self.m.entities {
            if !id.starts_with("ENT_MODE_") {
                continue;
            }
            let v = plain(e);
            let mut r = Map::new();
            for (k, x) in &v {
                if let Some(n) = k.strip_prefix("rules_") {
                    r.insert(n.to_string(), x.clone());
                }
            }
            let gm = r.get("game_mode").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let gs = r.get("game_state").and_then(|v| v.as_str()).unwrap_or("").to_string();
            rules.insert(format!("{gm}|{gs}"), Value::Object(r));
        }
        root.insert("mode_rules".into(), Value::Object(rules));
        let mut tree = Value::Object(root);
        round_f32(&mut tree);
        load_records(tree)
    }

    /// CombatConstants (group COMBAT of the matrix: the .rdata f32 at each va) + the FRichCurve 1/3 + bone FNames
    fn constants(&self) -> Result<Value> {
        let mut o = Map::new();
        for (id, e) in &self.m.entities {
            if let Some(n) = id.strip_prefix("ENT_CONST_COMBAT_") {
                if let Some(v) = e.values.get("FLD_CONST_VALUE") {
                    o.insert(n.to_string(), v.clone());
                }
            }
        }
        let rdata = Rdata::load(&self.native_dir.join("rdata.tsv"))?;
        // FRichCurve::EvalForTwoKeys rva 0x3024fa0: 1/3 at _DAT_14406a414 (CombatData._two_keys)
        o.insert("curve_third".into(), json!(rdata.f32(0x14406a414)?));
        // bone FNames: string va -> "`dynamic initializer for 'NAME_<x>''" (CombatConstants.fname)
        let names = |vas: &[u64]| -> Result<Vec<String>> { vas.iter().map(|va| rdata.fname(*va)).collect() };
        o.insert("head_bones".into(), json!(names(&[0x144317d48])?));
        o.insert("right_leg_bones".into(), json!(names(&[0x144317ba0, 0x144317b90, 0x144317bb0])?));
        o.insert("left_leg_bones".into(), json!(names(&[0x144317bd0, 0x144317bc0, 0x144317bd8])?));
        o.insert("kick_tier_bone".into(), json!(rdata.fname(0x144317ba0)?));
        Ok(Value::Object(o))
    }

    /// CombatData.curve_keys + curve_extrap: the CurveFloat export's FloatCurve
    fn curve(&self, path: &str) -> Result<Value> {
        let ex = self.rd.read(path).ok_or_else(|| format!("pak: no curve {path}"))?;
        let e = mh_pak::pkg::export_of(&ex, "CurveFloat").ok_or_else(|| format!("{path}: no CurveFloat export"))?;
        let fc = &e["Properties"]["FloatCurve"];
        let tail = |v: &Value| v.as_str().map(|s| s.rsplit("::").next().unwrap_or(s).to_string()).unwrap_or_else(|| "RCCE_Constant".into());
        Ok(json!({"keys": fc["Keys"].clone(), "pre": tail(&fc["PreInfinityExtrap"]), "post": tail(&fc["PostInfinityExtrap"])}))
    }

    /// PDB base chain (NativeCtor.layout(cls).base): `class X : public Y` of extract/native/types/X.h
    fn class_chain(&self, cls: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut cur = cls.to_string();
        for _ in 0..32 {
            if cur.is_empty() {
                break;
            }
            out.push(cur.clone());
            let h = std::fs::read_to_string(self.native_dir.join("types").join(format!("{cur}.h"))).unwrap_or_default();
            cur = h
                .lines()
                .find_map(|l| {
                    let l = l.trim();
                    l.strip_prefix(&format!("class {cur} : public ")).map(|r| r.split_whitespace().next().unwrap_or("").to_string())
                })
                .unwrap_or_default();
        }
        out
    }

    /// BP_MordhauCharacter BlockCollider template (CombatData.block_collider): BoxExtent / RelativeLocation, here in the
    /// reference's Godot convention (UePhysics.swap x 0.01) for data::Spec; physics.rs keeps UE cm
    fn block_collider(&self) -> Result<Value> {
        let (half, centre) = crate::physics::block_collider_ue(self.rd)?;
        let g = |v: [f32; 3]| json!([(v[0] * 0.01) as f64, (v[2] * 0.01) as f64, (v[1] * 0.01) as f64]); // Godot Vector3 * 0.01 (f32)
        Ok(json!({"half": g(half), "centre": g(centre)}))
    }

    /// TraceStart / TraceEnd of the weapon's mesh (UePhysics.weapon_mesh: SkeletalMesh, else the first skin part's), in
    /// the reference's Godot convention for data::Spec (physics.rs keeps UE cm)
    fn tracer_sockets(&self, weapon: &str) -> Result<Value> {
        match crate::physics::weapon_sockets_ue(self.rd, weapon)? {
            Some((a, b)) => {
                let g = |v: [f32; 3]| json!([(v[0] * 0.01) as f64, (v[2] * 0.01) as f64, (v[1] * 0.01) as f64]); // Godot Vector3 * 0.01 (f32)
                Ok(json!({"start": g(a), "end": g(b)}))
            }
            None => Ok(json!({"start": null, "end": null})),
        }
    }
}

/// extract/native/rdata.tsv (scripts/rdata_consts.py over the hash-checked exe): va -> raw bytes / f32 / funcs
pub struct Rdata {
    rows: BTreeMap<u64, (String, String)>, // va -> (raw hex, funcs)
}

impl Rdata {
    pub fn load(p: &Path) -> Result<Rdata> {
        let txt = std::fs::read_to_string(p).map_err(|e| format!("{}: {e}", p.display()))?;
        let mut rows = BTreeMap::new();
        for l in txt.lines().skip(1) {
            let c: Vec<&str> = l.split('\t').collect();
            if c.len() < 9 {
                continue;
            }
            if let Ok(va) = u64::from_str_radix(c[0], 16) {
                rows.insert(va, (c[3].to_string(), c[8].to_string()));
            }
        }
        Ok(Rdata { rows })
    }
    /// the binary32 at va (raw little-endian bytes)
    pub fn f32(&self, va: u64) -> Result<f32> {
        let (raw, _) = self.rows.get(&va).ok_or_else(|| format!("rdata: no va {va:#x}"))?;
        let b = u32::from_str_radix(&raw[..8], 16).map_err(|e| e.to_string())?;
        Ok(f32::from_bits(b.swap_bytes()))
    }
    /// CombatConstants.fname: the string's initializer "`dynamic initializer for 'NAME_<x>''" -> "<x>"
    pub fn fname(&self, va: u64) -> Result<String> {
        let (_, funcs) = self.rows.get(&va).ok_or_else(|| format!("rdata: no va {va:#x}"))?;
        for f in funcs.split(';') {
            if let Some(i) = f.find("'NAME_") {
                return Ok(f[i + 6..].split('\'').next().unwrap_or("").to_string());
            }
        }
        Err(format!("rdata: va {va:#x} has no NAME_ initializer"))
    }
}

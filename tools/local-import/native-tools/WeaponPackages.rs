//! Read weapon Blueprint/CDO chains directly from original paks. No exported-data input.
//! Build against source-built mh-pak and serde_json; output is a local prerequisite,
//! never a completed native constructor/spec matrix.
use mh_pak::{pkg, Reader, Vfs};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::sync::Arc;

const ROOTS: [&str; 4] = ["MordhauWeapon", "MordhauShield", "FistsWeapon", "KickWeapon"];

fn strip(path: &str) -> String {
    match path.rsplit_once('.') {
        Some((base, index)) if index.parse::<usize>().is_ok() => base.into(),
        _ => path.into(),
    }
}

fn chain(rd: &Reader, path: &str) -> Result<(Vec<String>, String), String> {
    let mut chain = Vec::new();
    let mut path = strip(path);
    while path.starts_with("Mordhau/Content/") {
        if chain.contains(&path) || chain.len() >= 64 {
            return Err(format!("cyclic/excessive Blueprint inheritance at {path}"));
        }
        rd.try_open(&path).map_err(|e| e.0)?;
        chain.push(path.clone());
        let sup = rd.super_of(&path);
        let op = sup.get("ObjectPath").and_then(Value::as_str).unwrap_or("");
        if op.starts_with("Mordhau/Content/") {
            path = strip(op);
        } else if op.starts_with("/Script/") {
            let name = sup.get("ObjectName").and_then(Value::as_str).unwrap_or("");
            let name = name.split('\'').nth(1).unwrap_or("");
            if name.is_empty() { return Err(format!("missing native parent name for {path}")); }
            return Ok((chain, name.into()));
        } else {
            return Err(format!("missing/unsupported parent for {path}: {sup}"));
        }
    }
    Err(format!("not a content Blueprint: {path}"))
}

fn refs(v: &Value, out: &mut BTreeSet<String>) {
    match v {
        Value::Object(m) => {
            if let Some(Value::String(path)) = m.get("ObjectPath") {
                if path.starts_with("Mordhau/Content/") { out.insert(strip(path)); }
            }
            for value in m.values() { refs(value, out); }
        }
        Value::Array(a) => for value in a { refs(value, out); },
        _ => {}
    }
}

fn record(rd: &Reader, path: &str, bp: bool) -> Result<Value, String> {
    let (ch, native) = if bp { chain(rd, path)? } else { (vec![path.into()], String::new()) };
    let mut merged = Map::new();
    let mut maps: BTreeMap<String, BTreeMap<String, Value>> = BTreeMap::new();
    let mut packages = Vec::new();
    let mut errors = Vec::new();
    for name in ch.iter().rev() {
        let pk = rd.try_open(name).map_err(|e| e.0)?;
        // Every consumed original package entry must pass the original pak SHA1 check.
        rd.vfs.try_read(&format!("{name}.uasset"), true).map_err(|e| e.0)?;
        if !pk.uexp.is_empty() { rd.vfs.try_read(&format!("{name}.uexp"), true).map_err(|e| e.0)?; }
        pk.unsupported.borrow_mut().clear();
        let mut exports = Vec::new();
        for (i, export) in pk.exports.iter().enumerate() {
            if !bp || export.name.starts_with("Default__") || export.name == "SkeletalMeshComponent" || export.name == "ClashCollider" {
                let value = rd.export_json(&pk, i);
                exports.push(json!({"export_index":i,"record":value}));
                if export.name.starts_with("Default__") {
                    if let Some(Value::Object(properties)) = value.get("Properties") {
                        pkg::merge_into(&mut merged, properties.clone());
                        // Match CombatData.map_prop: inherited Attacks entries merge by key.
                        if let Some(Value::Array(entries)) = properties.get("Attacks") {
                            let map = maps.entry("Attacks".into()).or_default();
                            for entry in entries {
                                let key = entry.get("Key").and_then(Value::as_str)
                                    .ok_or_else(|| format!("invalid Attacks key in {name}"))?;
                                let value = entry.get("Value").ok_or_else(|| format!("missing Attacks value in {name}"))?;
                                map.insert(key.into(), value.clone());
                            }
                        }
                    }
                }
            }
        }
        if bp && !exports.iter().any(|e| e["record"]["Name"].as_str().unwrap_or("").starts_with("Default__")) {
            return Err(format!("missing class default object in ancestor {name}"));
        }
        for error in pk.unsupported.borrow().iter() { errors.push(json!({"package":name,"error":error})); }
        packages.push(json!({"package":name,"uasset_bytes":pk.uasset.len(),"uexp_bytes":pk.uexp.len(),"exports":exports}));
    }
    if bp && !packages.iter().any(|p| p["exports"].as_array().unwrap().iter().any(|e| e["record"]["Name"].as_str().unwrap_or("").starts_with("Default__"))) {
        return Err(format!("missing class default object in {path}"));
    }
    let mut references = BTreeSet::new();
    refs(&Value::Object(merged.clone()), &mut references);
    for entries in maps.values() { for value in entries.values() { refs(value, &mut references); } }
    if !bp { for package in &packages { refs(package, &mut references); } }
    Ok(json!({"package":path,"native_root":native,"chain_child_first":ch,
        "serialized_defaults":merged,"maps_merged":maps,"original_packages":packages,
        "references":references,"decode_errors":errors,"native_defaults_included":false}))
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let output = args.first().ok_or("usage: weapon-packages <new-output.json> [--include <package>]...")?;
    let mut included = BTreeSet::new();
    let mut index = 1;
    while index < args.len() {
        if args[index] != "--include" || index + 1 >= args.len() { return Err("expected --include <package>".into()); }
        let path = strip(&args[index + 1]);
        if !path.starts_with("Mordhau/Content/") || path.split('/').any(|part| matches!(part, "." | "..")) { return Err("invalid included package".into()); }
        included.insert(path);
        index += 2;
    }
    let vfs = Arc::new(Vfs::mount_default().map_err(|e| e.0)?);
    if !vfs.errors.is_empty() { return Err(format!("original pak mount errors: {:?}", vfs.errors)); }
    let rd = Reader::new(vfs);
    let candidates = rd.packages_of_class("Mordhau/Content", &["BlueprintGeneratedClass", "Function"]);
    let mut weapons = Vec::new();
    let mut errors = Vec::new();
    for path in &candidates {
        // Non-class Function packages have no BlueprintGeneratedClass export.
        let sup = rd.super_of(path);
        if sup.is_null() { continue; }
        match chain(&rd, path) {
            Ok((_, root)) if ROOTS.contains(&root.as_str()) => weapons.push(path.clone()),
            Ok(_) => {},
            Err(error) => errors.push(json!({"package":path,"phase":"inheritance","error":error})),
        }
    }
    rd.clear_cache();
    eprintln!("{} candidate packages; {} weapon Blueprints", candidates.len(), weapons.len());
    let mut queue: VecDeque<String> = weapons.iter().chain(included.iter()).cloned().collect();
    let mut records: BTreeMap<String, Value> = BTreeMap::new();
    let mut inspected = BTreeSet::new();
    let mut excluded = Vec::new();
    while let Some(path) = queue.pop_front() {
        if !inspected.insert(path.clone()) { continue; }
        let ty = mh_pak::first_export_class(&rd.vfs, &path);
        let bp = matches!(ty.as_str(), "BlueprintGeneratedClass" | "Function");
        let curve = matches!(ty.as_str(), "CurveFloat" | "CurveVector" | "CurveLinearColor");
        let is_weapon = weapons.binary_search(&path).is_ok();
        if bp && !is_weapon && !included.contains(&path) {
            if rd.super_of(&path).is_null() {
                excluded.push(json!({"package":path,"reason":"reference is not a combat BlueprintGeneratedClass"}));
                continue;
            }
            match chain(&rd, &path) {
                Ok((_, root)) if root.ends_with("Motion") || root.ends_with("AnimationProfile") => {},
                Ok((_, root)) => { excluded.push(json!({"package":path,"native_root":root,"reason":"reference is not a combat motion/profile"})); continue; },
                Err(error) => { errors.push(json!({"package":path,"phase":"dependency-inheritance","error":error})); continue; }
            }
        } else if !bp && !curve { excluded.push(json!({"package":path,"export_type":ty,"reason":"asset payload is not combat CDO/curve data"})); continue; }
        match record(&rd, &path, bp) {
            Ok(value) => {
                for reference in value["references"].as_array().unwrap() {
                    queue.push_back(reference.as_str().unwrap().into());
                }
                records.insert(path, value);
            }
            Err(error) => errors.push(json!({"package":path,"phase":"decode","error":error})),
        }
        rd.clear_cache();
    }
    let result = json!({"schema":1,"source":"original-paks","runtime_ready":false,
        "candidate_count":candidates.len(),"weapons":weapons,"records":records,"errors":errors,
        "included_packages":included,"excluded_references":excluded,
        "limits":["Serialized Blueprint deltas only; missing fields require original native constructors.",
                  "Animation/audio/mesh references are retained, their asset payloads are not copied."]});
    let data = serde_json::to_vec_pretty(&result).map_err(|e| e.to_string())?;
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&output).map_err(|e| e.to_string())?;
    file.write_all(&data).map_err(|e| e.to_string())?;
    eprintln!("{} combat package records; {} errors", result["records"].as_object().unwrap().len(), result["errors"].as_array().unwrap().len());
    Ok(())
}

fn main() {
    if let Err(error) = run() { eprintln!("weapon-packages: {error}"); std::process::exit(2); }
}

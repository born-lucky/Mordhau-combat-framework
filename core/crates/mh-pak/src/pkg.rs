//! Package-level queries the readers use (port of `ue_pkg_pak.gd` `UePkgPak` and the backend-neutral parts of
//! `pkg/ue_pkg.gd` `UePkg`: cdo, super_of, chain, defaults). Values are mdx json shaped (`reader.rs`).

use crate::asset::first_export_class;
use crate::reader::{strip_index, Reader};
use serde_json::{Map, Value};

impl Reader {
    pub fn exists(&self, pkg_path: &str) -> bool {
        self.vfs.has(&format!("{pkg_path}.uasset")) || self.vfs.has(&format!("{pkg_path}.umap"))
    }

    /// Raw bytes of a non-package file (config .ini, .ufont, ...) as an owned Vec (UePkg.file on the pak backend)
    pub fn file(&self, pth: &str) -> Option<Vec<u8>> {
        self.vfs.read(pth, false).map(|b| b.to_vec())
    }

    /// Packages (no extension) under `dir` whose first export's class is one of `classes`, sorted: the rows of
    /// extract/manifest.tsv (mdx list: path, size, class of the first export) that the same filter selects
    /// (UePkgPak.packages_of_class)
    pub fn packages_of_class(&self, dir: &str, classes: &[&str]) -> Vec<String> {
        let pre = format!("{}/", dir.to_lowercase());
        let mut out: Vec<String> = self
            .vfs
            .list()
            .filter(|p| p.ends_with(".uasset") && p.to_lowercase().starts_with(&pre))
            .map(|p| p[..p.len() - 7].to_string())
            .filter(|p| classes.contains(&first_export_class(&self.vfs, p).as_str()))
            .collect();
        out.sort();
        out
    }

    /// SuperStruct of the package's first BlueprintGeneratedClass export, decoding only that export
    /// (UePkgPak.super_of / UePkg.super_path); Null when there is none
    pub fn super_of(&self, pkg_path: &str) -> Value {
        let Some(a) = self.open(pkg_path) else { return Value::Null };
        for i in 0..a.exports.len() {
            if let Some(cn) = self.node(&a, a.exports[i].cls) {
                if self.node_name(&cn) == "BlueprintGeneratedClass" {
                    return self.export_json(&a, i).get("SuperStruct").cloned().unwrap_or(Value::Null);
                }
            }
        }
        Value::Null
    }

    /// Blueprint packages from obj_path up to (not including) the native parent, child first (UePkg.chain)
    pub fn chain(&self, obj_path: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut p = strip_index(obj_path).to_string();
        while p.starts_with("Mordhau/Content") && out.len() < 64 {
            if !self.exists(&p) {
                break;
            }
            out.push(p.clone());
            let s = self.super_of(&p);
            p = strip_index(s.get("ObjectPath").and_then(|v| v.as_str()).unwrap_or("")).to_string();
        }
        out
    }

    /// Class defaults merged over the Blueprint SuperStruct chain, parent first (UePkg.defaults): struct values merge
    /// per field (a child that sets only Windup keeps the parent's Release), arrays replace whole
    pub fn defaults(&self, obj_path: &str) -> Map<String, Value> {
        let mut out = Map::new();
        for p in self.chain(obj_path).iter().rev() {
            let Some(exports) = self.read(p) else { continue };
            if let Value::Object(d) = cdo(&exports) {
                merge_into(&mut out, d);
            }
        }
        out
    }
}

/// Properties of the package's class default object (export named Default__*) (UePkg.cdo); Null when none
pub fn cdo(exports: &[Value]) -> Value {
    for e in exports {
        if e.get("Name").and_then(|v| v.as_str()).is_some_and(|n| n.starts_with("Default__")) {
            return e.get("Properties").cloned().unwrap_or_else(|| Value::Object(Map::new()));
        }
    }
    Value::Null
}

/// The first export of a type ("BlueprintGeneratedClass", "DataTable", ...)
pub fn export_of<'a>(exports: &'a [Value], ty: &str) -> Option<&'a Value> {
    exports.iter().find(|e| e.get("Type").and_then(|v| v.as_str()) == Some(ty))
}

/// UePkg._merge: objects merge per key, anything else replaces
pub fn merge(a: Option<Value>, b: Value) -> Value {
    match (a, b) {
        (Some(Value::Object(mut a)), Value::Object(b)) => {
            merge_into(&mut a, b);
            Value::Object(a)
        }
        (_, b) => b,
    }
}

/// Merge `b` into `a` keeping `a`'s key order (a GDScript Dictionary assignment keeps the key's position)
pub fn merge_into(a: &mut Map<String, Value>, b: Map<String, Value>) {
    for (k, v) in b {
        match a.get_mut(&k) {
            Some(slot) => {
                let old = std::mem::take(slot);
                *slot = merge(Some(old), v);
            }
            None => {
                a.insert(k, v);
            }
        }
    }
}

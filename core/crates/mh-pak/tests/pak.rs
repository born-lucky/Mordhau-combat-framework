//! Port of godot/tests/test_pak.gd: the pak/package readers on the user's own Steam install, read-only. Pak index vs
//! extract/manifest.tsv (CUE4Parse's listing), entry hashes, compression census, packages vs their JSON.
//! Without an install (no $MORDHAU_DIR, no Steam default) or without extract/, each test prints SKIP and passes.

use mh_pak::{equiv, first_export_class, pkg, Reader, Vfs};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};
use std::time::Instant;

const LONGSWORD: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";

fn vfs() -> Option<Arc<Vfs>> {
    static V: OnceLock<Option<Arc<Vfs>>> = OnceLock::new();
    let v = V.get_or_init(|| match Vfs::mount_default() {
        Ok(v) => Some(Arc::new(v)),
        Err(e) => {
            eprintln!("SKIP: {e}");
            None
        }
    });
    v.clone()
}

fn root() -> Option<PathBuf> {
    let r = equiv::extract_root();
    if r.join("manifest.tsv").exists() {
        Some(r)
    } else {
        eprintln!("SKIP: no extract/manifest.tsv under {}", r.display());
        None
    }
}

/// extract/manifest.tsv rows: lower-case path -> (size, class)
fn manifest() -> HashMap<String, (u64, String)> {
    static M: OnceLock<HashMap<String, (u64, String)>> = OnceLock::new();
    M.get_or_init(|| {
        let mut out = HashMap::new();
        let Some(r) = root() else { return out };
        let t = std::fs::read_to_string(r.join("manifest.tsv")).unwrap_or_default();
        for l in t.lines().skip(1) {
            let c: Vec<&str> = l.split('\t').collect();
            if c.len() < 2 {
                continue;
            }
            // CUE4Parse lists the one file at the pak root as "/fastcook.txt"; the mounted path is "fastcook.txt"
            let p = c[0].to_lowercase();
            out.insert(p.trim_start_matches('/').to_string(), (c[1].parse().unwrap_or(0), c.get(2).unwrap_or(&"").to_string()));
        }
        out
    })
    .clone()
}

macro_rules! need {
    ($e:expr) => {
        match $e {
            Some(v) => v,
            None => return,
        }
    };
}

/// Every pak opens; the footer and index decode as UE 4.26 v11; the mounted file list equals mdx list's manifest
#[test]
fn pak_index_matches_manifest() {
    let v = need!(vfs());
    need!(root());
    assert!(v.errors.is_empty(), "pak errors: {:?}", v.errors);
    assert_eq!(v.paks.len(), 44, "mordhau.702625635.sha1 pins 44 paks");
    let mut total = 0;
    for pk in &v.paks {
        assert_eq!(pk.version, 11, "{}: expected 11 Fnv64BugFix", pk.file_name());
        assert_eq!(pk.compression.len(), 1, "{} names compression methods {:?}", pk.file_name(), pk.compression);
        total += pk.entry_count as usize;
    }
    let man = manifest();
    assert_eq!(total, man.len(), "pak entries vs manifest");
    assert_eq!(v.file_count(), man.len(), "mounted vs manifest");
    let missing = v.list().filter(|p| !man.contains_key(&p.to_lowercase())).count();
    assert_eq!(missing, 0, "mounted paths not in the manifest");
    println!("  {} paks, {} entries == manifest", v.paks.len(), total);
}

/// No entry is compressed or encrypted, so 100% of the packages are readable without Oodle (docs/PAK_FORMAT.md)
#[test]
fn pak_no_compressed_entries() {
    let v = need!(vfs());
    let c = v.census();
    assert_eq!(c.len(), 2, "census {c:?}");
    assert_eq!(c.get("method None").copied(), Some(v.file_count()), "census {c:?}");
    assert_eq!(c["encrypted"], 0);
    println!("  pak census: {} entries, all uncompressed and unencrypted", v.file_count());
}

/// Entry bytes: the size matches the manifest and the SHA1 the pak stores in the entry header matches the bytes
#[test]
fn pak_entry_sha1() {
    let v = need!(vfs());
    need!(root());
    let man = manifest();
    for p in [
        format!("{LONGSWORD}.uasset"),
        format!("{LONGSWORD}.uexp"),
        "Mordhau/Config/DefaultInput.ini".to_string(),
    ] {
        let b = v.try_read(&p, true).unwrap_or_else(|e| panic!("{p}: {e}"));
        assert_eq!(Some(b.len() as u64), man.get(&p.to_lowercase()).map(|m| m.0), "{p} size vs manifest");
    }
    // a corrupted hash would be caught: ranged reads stay inside the entry
    let b = v.read(&format!("{LONGSWORD}.uasset"), false).unwrap();
    let r = v.read_range(&format!("{LONGSWORD}.uasset"), 4, 8).unwrap();
    assert_eq!(&r[..], &b[4..12]);
    assert!(v.read_range(&format!("{LONGSWORD}.uasset"), b.len() as u64 - 2, 4).is_none());
}

/// The game's own config file from the pak equals the copy `mdx raw` wrote to extract/config
#[test]
fn pak_config_equals_extract() {
    let v = need!(vfs());
    let r = need!(root());
    let b = v.read("Mordhau/Config/DefaultInput.ini", false).unwrap();
    let f = std::fs::read(r.join("config/DefaultInput.ini")).expect("extract/config/DefaultInput.ini");
    assert!(b[..] == f[..], "DefaultInput.ini differs between pak and extract/config");
}

/// Package summary, name/import/export maps and every export of BP_Longsword equal extract/json exactly
#[test]
fn asset_longsword_equals_json() {
    let v = need!(vfs());
    let r = need!(root());
    let rd = Reader::new(v);
    let mut pe = equiv::Equiv::default();
    pe.compare_package(&rd, &r, LONGSWORD);
    assert_eq!(pe.packages_exact, 1, "{} first: {:?}", pe.summary(), &pe.mismatches[..pe.mismatches.len().min(3)]);
    let a = rd.open(LONGSWORD).unwrap();
    assert_eq!((a.names.len(), a.imports.len(), a.exports.len()), (341, 183, 10));
    // a trailing export index is dropped, any case is accepted, the stored spelling is kept
    let b = rd.open(&format!("{}.3", LONGSWORD.to_lowercase())).unwrap();
    assert_eq!(b.name, LONGSWORD);
}

/// The pak reader's export list has the shape the JSON backend gives: same CDO keys, same values the readers use
#[test]
fn backend_switch_reads_pak() {
    let v = need!(vfs());
    let r = need!(root());
    let rd = Reader::new(v);
    let a = rd.read(&format!("{LONGSWORD}.0")).unwrap();
    let j = equiv::json_of(&r, LONGSWORD);
    assert!(!a.is_empty() && a.len() == j.len(), "export count pak {} json {}", a.len(), j.len());
    let (cp, cj) = (pkg::cdo(&a), pkg::cdo(&j));
    let keys = |c: &Value| c.as_object().map(|m| m.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
    assert_eq!(keys(&cp), keys(&cj), "CDO keys differ");
    assert_eq!(cp["EquipmentName"]["SourceString"], "Longsword");
    let sj = pkg::export_of(&j, "BlueprintGeneratedClass").unwrap()["SuperStruct"]["ObjectPath"].clone();
    assert_eq!(rd.super_of(LONGSWORD)["ObjectPath"], sj);
}

/// Blueprint class defaults merged over the SuperStruct chain (UePkg.defaults) equal the same merge over extract/json,
/// beyond CUE4Parse's NetQuantize misread
#[test]
fn backend_pak_defaults_equal_json() {
    let v = need!(vfs());
    let r = need!(root());
    let rd = Reader::new(v);
    let mut pe = equiv::Equiv::default();
    for p in [
        LONGSWORD,
        "Mordhau/Content/Mordhau/Blueprints/Characters/BP_MordhauCharacter",
        "Mordhau/Content/Mordhau/Blueprints/GameModes/Duel/BP_DuelGameMode",
    ] {
        let a = rd.defaults(p);
        // the same merge over the JSON copies (UePkg.defaults on the json backend)
        let mut chain = vec![];
        let mut q = p.to_string();
        while q.starts_with("Mordhau/Content") {
            let js = equiv::json_of(&r, &q);
            if js.is_empty() {
                break;
            }
            chain.push(js.clone());
            let s = pkg::export_of(&js, "BlueprintGeneratedClass").and_then(|e| e["SuperStruct"]["ObjectPath"].as_str()).unwrap_or("");
            q = mh_pak::reader::strip_index(s).to_string();
        }
        let mut j = serde_json::Map::new();
        for js in chain.iter().rev() {
            if let Value::Object(d) = pkg::cdo(js) {
                pkg::merge_into(&mut j, d);
            }
        }
        assert!(!a.is_empty() && a.len() == j.len(), "{p}: {} keys from pak, {} from json", a.len(), j.len());
        assert_eq!(rd.chain(p).len(), chain.len(), "{p}: chain length");
        pe.eq(p, "defaults", &Value::Object(a), &Value::Object(j));
    }
    let (_, real) = pe.split_misreads();
    assert!(real.is_empty(), "{:?}", &real[..real.len().min(3)]);
}

/// The class of every package's first export, read from the .uasset header alone, equals the class column mdx list
/// (CUE4Parse) wrote to extract/manifest.tsv, for every .uasset under Mordhau/Content
#[test]
fn first_export_class_matches_manifest() {
    let v = need!(vfs());
    need!(root());
    let man = manifest();
    let t = Instant::now();
    let mut rows: Vec<(&String, &(u64, String))> =
        man.iter().filter(|(p, _)| p.starts_with("mordhau/content/") && p.ends_with(".uasset")).collect();
    rows.sort();
    let mut bad = vec![];
    for (p, (_, cls)) in &rows {
        let sp = v.spelling(p);
        let c = first_export_class(&v, &sp[..sp.len() - 7]);
        if &c != cls {
            bad.push(format!("{sp}: {c} vs manifest {cls}"));
        }
    }
    println!("  {} packages, {:.1} s", rows.len(), t.elapsed().as_secs_f64());
    assert!(rows.len() >= 35000, "only {} packages", rows.len());
    assert!(bad.is_empty(), "{} differ, first {:?}", bad.len(), &bad[..bad.len().min(3)]);
}

/// The listings the readers scan (weapons, wearables) are the same from the paks and from the manifest
#[test]
fn packages_of_class_pak_equals_manifest() {
    let v = need!(vfs());
    need!(root());
    let rd = Reader::new(v.clone());
    let man = manifest();
    for d in ["Mordhau/Content/Mordhau/Blueprints/Wearables", "Mordhau/Content/Mordhau/Blueprints/Equipment"] {
        let classes = ["BlueprintGeneratedClass", "Function"];
        let a = rd.packages_of_class(d, &classes);
        let pre = format!("{}/", d.to_lowercase());
        let mut j: Vec<String> = man
            .iter()
            .filter(|(p, (_, c))| p.starts_with(&pre) && p.ends_with(".uasset") && classes.contains(&c.as_str()))
            .map(|(p, _)| {
                let s = v.spelling(p);
                s[..s.len() - 7].to_string()
            })
            .collect();
        j.sort();
        assert!(!j.is_empty() && a == j, "{d}: {} from paks, {} from manifest", a.len(), j.len());
    }
}

/// Config files and font faces are the same bytes from the paks and from the extract copies (UePkg.file)
#[test]
fn files_pak_equal_extract() {
    let v = need!(vfs());
    let r = need!(root());
    let rd = Reader::new(v);
    for pth in [
        "Mordhau/Config/DefaultInput.ini",
        "Mordhau/Config/DefaultEngine.ini",
        "Engine/Config/BaseEngine.ini",
        "Mordhau/Content/Mordhau/UI/UIAssets/Fonts/Cinzel-Regular_new.ufont",
    ] {
        let a = rd.file(pth).unwrap_or_default();
        // json backend: extract/config/<file> for Config folders, else extract/raw/<path> (UePkg.file)
        let name = pth.rsplit('/').next().unwrap();
        let c = r.join("config").join(name);
        let base_is_config = pth.rsplit_once('/').unwrap().0.ends_with("Config");
        let j = if base_is_config && c.exists() { std::fs::read(c) } else { std::fs::read(r.join("raw").join(pth)) }.unwrap_or_default();
        assert!(!a.is_empty() && a == j, "{pth}: {} bytes from paks, {} from extract", a.len(), j.len());
    }
}

/// FByteBulkData of a Texture2D's first mip is located inside the package files and read by range
#[test]
fn bulk_mip_header_located() {
    let v = need!(vfs());
    let rd = Reader::new(v.clone());
    // the first Texture2D package under UI/ (UeTexture's layout: props, Guid, 2+2 strip flags, bCooked, PixelFormat
    // FName, SkipOffset, SizeX, SizeY, PackedData, FString, FirstMip, mip count, then per mip bCooked + bulk header)
    let mut found = 0;
    let mut cands: Vec<String> = v
        .list()
        .filter(|p| p.starts_with("Mordhau/Content/Mordhau/UI/") && p.ends_with(".uasset"))
        .map(|p| p[..p.len() - 7].to_string())
        .collect();
    cands.sort();
    for p in cands {
        if first_export_class(&v, &p) != "Texture2D" {
            continue;
        }
        let a = rd.open(&p).unwrap();
        let e = &a.exports[0];
        let mut r = a.cursor();
        r.p = e.off;
        rd.tagged(&a, &mut r, e.off + e.size);
        if e.flags & mh_pak::asset::RF_CLASS_DEFAULT_OBJECT == 0 && r.s32() != 0 {
            r.skip(16);
        }
        r.skip(4);
        assert_ne!(r.s32(), 0, "{p}: not cooked");
        let pf = a.fname(&mut r);
        assert!(pf.starts_with("PF_"), "{p}: pixel format {pf}");
        r.skip(8);
        let (sx, sy, packed) = (r.s32(), r.s32(), r.u32());
        let _ = r.fstring();
        if packed & (1 << 30) != 0 {
            r.skip(8);
        }
        r.s32();
        let n = r.s32();
        assert!(n > 0 && n <= 32, "{p}: {n} mips");
        r.s32();
        let h = mh_pak::bulk::header(&a, &mut r);
        let mip0 = (r.s32(), r.s32());
        assert_eq!(mip0, (sx, sy), "{p}: mip 0 size");
        let b = mh_pak::bulk::bytes(&v, &a, &h).unwrap_or_else(|| panic!("{p}: payload {h:?}"));
        assert_eq!(b.len() as i64, h.size);
        found += 1;
        if found == 20 {
            break;
        }
    }
    assert!(found > 0, "no Texture2D under UI/");
}

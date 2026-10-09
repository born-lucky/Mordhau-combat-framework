// Constants cross-check (sheets r9): every .rdata literal compiled into the Rust crates (raw bits written in the
// source next to their .rdata address) equals the spec's constant entity at that address (data_gen/spec
// entities/constant.json: FLD_CONST_VA + FLD_CONST_VALUE / VALUE_F64), bit for bit as f32 / f64.
// Forms read from the crates' sources (text, no crate dependency):
//   - tuples `("name", 0x14xxxxxxx, 0x<bits>, &[..])` (mh-net consts.rs CITES / CITES64)
//   - a `/// .rdata 0x14xxxxxxx = ...` doc line followed by `f32::from_bits(0x<bits>)` (mh-mode consts.rs)
//   - one line holding both `f32::from_bits(0x<bits>)` and `.rdata 0x14xxxxxxx` (inline literals, mh-character)
// An address the spec has no constant for is listed (the spec covers the reference's named constants; engine-only
// literals of the Rust port are not rows yet) and checked against extract/native/rdata.tsv when that file is present.
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn repo() -> PathBuf {
    std::env::var("MORDHAU_REPO").map(PathBuf::from).unwrap_or_else(|_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.."))
}

fn hex(s: &str) -> Option<u64> {
    u64::from_str_radix(&s.trim_start_matches("0x").replace('_', ""), 16).ok()
}

/// (file:line, va, bits, width 32/64) of every literal in the sources
fn literals() -> Vec<(String, u64, u64, u8)> {
    let mut out = vec![];
    let crates = repo().join("core/crates");
    let mut files = vec![];
    for c in std::fs::read_dir(&crates).unwrap().flatten() {
        let src = c.path().join("src");
        let mut stack = vec![src];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() { stack.push(p) } else if p.extension().is_some_and(|x| x == "rs") { files.push(p) }
            }
        }
    }
    files.sort();
    let num = |s: &str| -> Vec<String> {
        // every 0x... token of a line
        let mut v = vec![];
        let b = s.as_bytes();
        let mut i = 0;
        while i + 2 < b.len() {
            if b[i] == b'0' && b[i + 1] == b'x' {
                let j = (i + 2..b.len()).find(|&k| !(b[k].is_ascii_hexdigit() || b[k] == b'_')).unwrap_or(b.len());
                v.push(s[i..j].to_string());
                i = j;
            } else {
                i += 1;
            }
        }
        v
    };
    for p in files {
        let rel = p.strip_prefix(repo()).unwrap_or(&p).display().to_string().replace('\\', "/");
        let text = std::fs::read_to_string(&p).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        for (i, l) in lines.iter().enumerate() {
            let code = l.trim_start();
            let at = format!("{rel}:{}", i + 1);
            // tuple form ("name", va, bits, ...)
            if code.starts_with("(\"") {
                let t = num(l);
                if t.len() >= 2 {
                    if let (Some(va), Some(bits)) = (hex(&t[0]), hex(&t[1])) {
                        if (0x140000000..0x150000000).contains(&va) {
                            out.push((at.clone(), va, bits, if t[1].trim_start_matches("0x").replace('_', "").len() > 8 { 64 } else { 32 }));
                            continue;
                        }
                    }
                }
            }
            // doc line + next line from_bits
            if code.starts_with("/// .rdata 0x") {
                let va = num(l).first().and_then(|s| hex(s));
                let next = lines.get(i + 1).copied().unwrap_or("");
                if let (Some(va), Some(pos)) = (va, next.find("f32::from_bits(")) {
                    if let Some(bits) = num(&next[pos..]).first().and_then(|s| hex(s)) {
                        out.push((format!("{rel}:{}", i + 2), va, bits, 32));
                    }
                }
                continue;
            }
            // inline: from_bits + ".rdata 0x14..." on one line
            if let (Some(pf), Some(pr)) = (l.find("f32::from_bits("), l.find(".rdata 0x")) {
                let bits = num(&l[pf..]).first().and_then(|s| hex(s));
                let va = num(&l[pr..]).first().and_then(|s| hex(s));
                if let (Some(bits), Some(va)) = (bits, va) {
                    out.push((at, va, bits, 32));
                }
            }
        }
    }
    out
}

#[test]
fn compiled_constants_equal_the_spec() {
    let dir = mh_spec::Spec::default_dir();
    if !dir.join("index.json").exists() {
        eprintln!("SKIP: no {}", dir.display());
        return;
    }
    let spec = mh_spec::Spec::load(&dir, false).expect("spec");
    // va -> (f32 bits, f64 bits) of the spec constants
    let mut by_va: BTreeMap<u64, (Option<u32>, Option<u64>, String)> = BTreeMap::new();
    for id in spec.entities_of("constant") {
        let e = &spec.entities[id];
        let Some(va) = e.values.get("FLD_CONST_VA").and_then(|v| v.as_str()).and_then(hex) else { continue };
        let f = spec.f32(id, "FLD_CONST_VALUE").ok().map(|x| x.to_bits());
        let d = spec.f64(id, "FLD_CONST_VALUE_F64").ok().map(|x| x.to_bits());
        let slot = by_va.entry(va).or_insert((None, None, id.to_string()));
        slot.0 = slot.0.or(f);
        slot.1 = slot.1.or(d);
    }
    // rdata.tsv raw column (exe bytes) for addresses the spec does not have
    let mut raw: BTreeMap<u64, String> = BTreeMap::new();
    if let Ok(t) = std::fs::read_to_string(repo().join("extract/native/rdata.tsv")) {
        for l in t.lines().skip(1) {
            let c: Vec<&str> = l.split('\t').collect();
            if c.len() > 3 {
                if let Some(va) = hex(c[0]) { raw.insert(va, c[3].to_string()); }
            }
        }
    }
    let lits = literals();
    assert!(lits.len() > 50, "only {} literals found: the source patterns changed?", lits.len());
    let (mut in_spec, mut exe_only, mut unknown) = (0, 0, vec![]);
    let mut fails = vec![];
    for (at, va, bits, w) in &lits {
        match by_va.get(va) {
            Some((f, d, id)) => {
                in_spec += 1;
                let ok = if *w == 64 { d.is_some_and(|d| d == *bits) } else { f.is_some_and(|f| f as u64 == *bits) };
                if !ok { fails.push(format!("{at}: 0x{va:x} bits 0x{bits:x} vs spec {id} {f:x?}/{d:x?}")) }
            }
            None => match raw.get(va) {
                Some(r) => {
                    exe_only += 1;
                    // the raw column is the little-endian bytes as hex (8 bytes); the literal is the first 4 / 8
                    let le = (0..r.len() / 2).filter_map(|i| u8::from_str_radix(&r[2 * i..2 * i + 2], 16).ok()).collect::<Vec<_>>();
                    let v = if *w == 64 && le.len() >= 8 { u64::from_le_bytes(le[..8].try_into().unwrap()) }
                        else if le.len() >= 4 { u32::from_le_bytes(le[..4].try_into().unwrap()) as u64 } else { u64::MAX };
                    if v != *bits { fails.push(format!("{at}: 0x{va:x} bits 0x{bits:x} vs exe .rdata 0x{v:x}")) }
                }
                None => unknown.push(format!("{at} 0x{va:x}")),
            },
        }
    }
    eprintln!("constants: {} literals, {in_spec} == the spec's constant at that va, {exe_only} not spec rows (== exe .rdata), {} unchecked", lits.len(), unknown.len());
    if !unknown.is_empty() { eprintln!("  unchecked (no spec row, no rdata.tsv row): {:?}", &unknown[..unknown.len().min(10)]); }
    assert!(fails.is_empty(), "{} mismatch(es): {:?}", fails.len(), &fails[..fails.len().min(10)]);
    assert!(in_spec > 50, "only {in_spec} literals matched a spec constant");
}

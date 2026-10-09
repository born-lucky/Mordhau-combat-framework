//! mh-pak - the game's paks from the command line (read-only; the install is $MORDHAU_DIR or Steam's default).
//!   mh-pak list [prefix]          mounted paths (stored spelling), optionally filtered by a case-insensitive prefix
//!   mh-pak json <pkg>             exports of a package in extract/json's shape ("Mordhau/Content/.../BP_X", no ext)
//!   mh-pak raw <path> [out]       bytes of any pak file to stdout or to `out` (SHA1-checked)
//!   mh-pak info                   paks, versions, entry count, compression census
//!   mh-pak equiv [--full]         equivalence vs extract/json (used packages, or every Blueprints/ + UI/ package)

use mh_pak::{equiv, Reader, Vfs};
use std::io::Write;
use std::sync::Arc;
use std::time::Instant;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map(|s| s.as_str()).unwrap_or("");
    if !matches!(cmd, "list" | "json" | "raw" | "info" | "equiv") {
        eprintln!("usage: mh-pak list [prefix] | json <pkg> | raw <path> [out] | info | equiv [--full]");
        std::process::exit(2);
    }
    let vfs = match Vfs::mount_default() {
        Ok(v) => Arc::new(v),
        Err(e) => {
            eprintln!("mh-pak: {e}");
            std::process::exit(1);
        }
    };
    let out = std::io::stdout();
    let mut out = out.lock();
    match cmd {
        "list" => {
            let pre = args.get(1).map(|s| s.to_lowercase()).unwrap_or_default();
            let mut v: Vec<&str> = vfs.list().filter(|p| p.to_lowercase().starts_with(&pre)).collect();
            v.sort();
            for p in v {
                let _ = writeln!(out, "{p}");
            }
        }
        "json" => {
            let Some(p) = args.get(1) else { die("json <pkg>") };
            let rd = Reader::new(vfs.clone());
            match rd.try_open(p) {
                Ok(_) => {
                    let ex = rd.read(p).unwrap_or_default();
                    let _ = writeln!(out, "{}", serde_json::to_string_pretty(&ex).unwrap());
                }
                Err(e) => die(&e.0),
            }
        }
        "raw" => {
            let Some(p) = args.get(1) else { die("raw <path> [out]") };
            let b = vfs.try_read(p, true).unwrap_or_else(|e| die(&e.0));
            match args.get(2) {
                Some(f) => std::fs::write(f, &b[..]).unwrap_or_else(|e| die(&e.to_string())),
                None => {
                    let _ = out.write_all(&b);
                }
            }
        }
        "info" => {
            for pk in &vfs.paks {
                let _ = writeln!(out, "{}\tv{}\t{}\t{} entries\t{:?}", pk.file_name(), pk.version, pk.mount_point, pk.entry_count, pk.compression);
            }
            let _ = writeln!(out, "{} paks, {} mounted files, census {:?}", vfs.paks.len(), vfs.file_count(), vfs.census());
            for e in &vfs.errors {
                let _ = writeln!(out, "error: {e}");
            }
        }
        "equiv" => {
            let full = args.iter().any(|a| a == "--full");
            let rd = Reader::new(vfs.clone());
            let root = equiv::extract_root();
            let t = Instant::now();
            let pe = if full { equiv::run(&rd, &root, equiv::FULL, &[]) } else { equiv::run(&rd, &root, equiv::SETS, equiv::SINGLES) };
            let (known, real) = pe.split_misreads();
            let _ = writeln!(out, "{:.1} s", t.elapsed().as_secs_f64());
            let _ = writeln!(out, "pak vs json: {}; {} of them CUE4Parse NetQuantize misreads", pe.summary(), known.len());
            let _ = writeln!(out, "undecoded by export type: {:?}", pe.unsupported_types);
            let _ = writeln!(out, "undecoded by reason: {:?}", pe.unsupported_reasons);
            for m in real.iter().take(40) {
                let _ = writeln!(out, "  MISMATCH {} {}: pak={} json={}", m.pkg, m.path, trunc(&m.pak), trunc(&m.json));
            }
            if !real.is_empty() {
                std::process::exit(1);
            }
        }
        _ => unreachable!(),
    }
}

fn trunc(v: &serde_json::Value) -> String {
    let s = v.to_string();
    if s.len() > 160 { format!("{}...", &s[..s.char_indices().nth(160).map_or(s.len(), |c| c.0)]) } else { s }
}

fn die(msg: &str) -> ! {
    eprintln!("mh-pak: {msg}");
    std::process::exit(1)
}

//! mh-parity CLI (the Rust backend of scripts/parity/compare.py; same input/output files as replay.gd):
//!   mh-parity probe|replay --mode compat|exe <records> <in.json> <out.json> [from count]
//!   mh-parity diff <compare_a.json> <compare_b.json> <summary_out.json> [--tol x]
//! <records> (source.rs):
//!   --matrix [--layer <mod_layer.json>]  the spec matrix data_gen/spec (+ paks) through mh-sim's SpecBuilder, plus the
//!                                         mod layer on modded servers (exe; Godot-free)
//!   --records <dump.json>                 a GDScript record dump (data_gen/parity/records_<server>.json or an
//!                                         export_golden.gd --spec-only file): compat's source, also usable by exe
//! compat = World::new_reference on the dump's f64 values; exe = World::new on f32 values (combat/exe.rs).
//! The last stdout line of probe / replay is "PARITY_DONE <entries>", like replay.gd.

use mh_parity::{diff_compare, source, Mode, Prober};
use serde_json::Value;
use std::io::Write;
use std::rc::Rc;

fn read_json(p: &str) -> Value {
    let s = std::fs::read_to_string(p).unwrap_or_else(|e| panic!("read {p}: {e}"));
    serde_json::from_str(&s).unwrap_or_else(|e| panic!("parse {p}: {e}"))
}

fn usage() -> ! {
    eprintln!("usage: mh-parity probe|replay --mode compat|exe (--matrix [--layer l.json] | --records r.json) <in.json> <out.json> [from count]\n       mh-parity diff <a.json> <b.json> <summary.json> [--tol x]
       mh-parity traced <weapons.json> <out.json> [--dt s]");
    std::process::exit(2)
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        usage();
    }
    let cmd = args[0].as_str();
    let mut mode = Mode::Exe;
    let mut spec_path = String::new();
    let mut matrix = false;
    let mut layer: Option<String> = None;
    let mut tol = 1e-6;
    let mut dt_arg = 0.001f64;
    let mut pos = Vec::new();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--mode" => {
                mode = match args.get(i + 1).map(|s| s.as_str()) {
                    Some("compat") => Mode::Compat,
                    Some("exe") => Mode::Exe,
                    _ => usage(),
                };
                i += 2;
            }
            "--matrix" => {
                matrix = true;
                i += 1;
            }
            "--layer" => {
                layer = args.get(i + 1).cloned();
                i += 2;
            }
            "--spec" | "--records" => {
                spec_path = args.get(i + 1).cloned().unwrap_or_else(|| usage());
                i += 2;
            }
            "--dt" => {
                dt_arg = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or_else(|| usage());
                i += 2;
            }
            "--tol" => {
                tol = args.get(i + 1).and_then(|s| s.parse().ok()).unwrap_or_else(|| usage());
                i += 2;
            }
            _ => {
                pos.push(args[i].clone());
                i += 1;
            }
        }
    }
    if cmd == "pakraw" {
        // mh-parity pakraw <mod.pak> <path substring> <out dir>: raw .uasset/.uexp (+ .ubulk) of a server mod's pak
        // in the game's own format (v11, mh_pak::Pak), read-only, for the Kismet decoder / curve reader
        // (scripts/parity/modpak_raw.py reads the older v5 NoChamber pak)
        if pos.len() < 3 {
            usage();
        }
        let mut files = Vec::new();
        let pak = mh_pak::Pak::open(std::path::Path::new(&pos[0]), &mut files).unwrap_or_else(|e| panic!("{e:?}"));
        let mut n = 0;
        for (p, loc) in &files {
            if !p.contains(pos[1].as_str()) || !(p.ends_with(".uasset") || p.ends_with(".uexp") || p.ends_with(".ubulk")) {
                continue;
            }
            let b = pak.read(*loc, true).unwrap_or_else(|e| panic!("{p}: {e:?}"));
            // mounted paths start with the pak's mount point ("../../../Mordhau/..."): keep only the part from the
            // first normal component, so nothing is written outside <out dir>
            let rel: std::path::PathBuf = std::path::Path::new(p)
                .components()
                .filter(|c| matches!(c, std::path::Component::Normal(_)))
                .collect();
            let out = std::path::Path::new(&pos[2]).join(rel);
            std::fs::create_dir_all(out.parent().unwrap()).unwrap();
            std::fs::write(&out, &*b).unwrap();
            n += 1;
        }
        println!("pakraw: {n} files of {}", files.len());
        return;
    }
    if cmd == "animdiag" {
        // mh-parity animdiag <weapon path> <move> <dist cm> <out.json>: traced.rs Traced::diag for the graph and the clip
        if pos.len() < 4 {
            usage();
        }
        let recs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json");
        let t = mh_parity::traced::Traced::new(&recs, dt_arg as f32, mh_parity::traced::Anim::Graph).unwrap_or_else(|e| panic!("{e}"));
        let mv: i64 = pos[1].parse().unwrap_or(0);
        let d: f32 = pos[2].parse().unwrap_or(90.0);
        let g = t.diag(&pos[0], mv, d, mh_parity::traced::Anim::Graph).unwrap_or_else(|e| panic!("{e}"));
        let c = t.diag(&pos[0], mv, d, mh_parity::traced::Anim::Clip).unwrap_or_else(|e| panic!("{e}"));
        std::fs::write(&pos[3], serde_json::to_string(&serde_json::json!({"graph": g, "clip": c})).unwrap()).expect("write");
        return;
    }
    if cmd == "turnsweep" {
        // mh-parity turnsweep <weapons.json> <out.json> [--dt s]: traced.rs Traced::turn_sweep (graph) per weapon
        if pos.len() < 2 {
            usage();
        }
        let recs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json");
        let t = mh_parity::traced::Traced::new(&recs, dt_arg as f32, mh_parity::traced::Anim::Graph).unwrap_or_else(|e| panic!("{e}"));
        let doc = read_json(&pos[0]);
        let mut out = serde_json::Map::new();
        for w in doc["weapons"].as_array().cloned().unwrap_or_default() {
            let w = w.as_str().unwrap_or("").to_string();
            match t.turn_sweep(&w) {
                Ok(v) => {
                    out.insert(w, v);
                }
                Err(e) => eprintln!("turnsweep {w}: {e}"),
            }
        }
        std::fs::write(&pos[1], serde_json::to_string(&Value::Object(out.clone())).unwrap()).expect("write");
        println!("PARITY_DONE {}", out.len());
        return;
    }
    if cmd == "traced" {
        // mh-parity traced <weapons.json> <out.json> [--dt s]: traced-hit metrics (traced.rs) per weapon; the
        // character movement records are core/tests/golden/character/records.json (mh-character)
        if pos.len() < 2 {
            usage();
        }
        let doc = read_json(&pos[0]);
        let recs = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/golden/character/records.json");
        let anim = if args.iter().any(|a| a == "--anim-clip") { mh_parity::traced::Anim::Clip } else { mh_parity::traced::Anim::Graph };
        let t = mh_parity::traced::Traced::new(&recs, dt_arg as f32, anim).unwrap_or_else(|e| panic!("traced: {e}"));
        let mut out = serde_json::Map::new();
        for w in doc["weapons"].as_array().into_iter().flatten() {
            let w = w.as_str().unwrap_or("");
            let parriers: Vec<String> = doc["parriers"][w]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            match t.probe_weapon(w, &parriers) {
                Ok(v) => {
                    out.insert(w.to_string(), v);
                }
                Err(e) => eprintln!("traced {w}: {e}"),
            }
        }
        std::fs::write(&pos[1], serde_json::to_string(&Value::Object(out.clone())).unwrap()).expect("write");
        println!("PARITY_DONE {}", out.len());
        return;
    }
    if cmd == "diff" {
        if pos.len() < 3 {
            usage();
        }
        let d = diff_compare(&read_json(&pos[0]), &read_json(&pos[1]), tol);
        std::fs::write(&pos[2], serde_json::to_string_pretty(&d).unwrap()).expect("write summary");
        println!("rows {} vs {}: status changed {}, value changed {}, unmatched {}, PASS->FAIL {}, FAIL->PASS {}, equivalent {}",
            d["rows_a"], d["rows_b"], d["status_changed"], d["value_changed"], d["unmatched"], d["pass_to_fail"],
            d["fail_to_pass"], d["equivalent"]);
        return;
    }
    if (cmd != "probe" && cmd != "replay") || pos.len() < 2 || (spec_path.is_empty() && !matrix) {
        usage();
    }
    if matrix && mode == Mode::Compat {
        eprintln!("--mode compat needs --records (the reference's f64 values; source.rs)");
        std::process::exit(2);
    }
    let doc = read_json(&pos[0]);
    let spec = if matrix {
        source::matrix_spec(&source::weapons_of(cmd, &doc), layer.as_deref().map(std::path::Path::new))
    } else {
        source::records_spec(std::path::Path::new(&spec_path), mode == Mode::Exe)
    }
    .unwrap_or_else(|e| panic!("records: {e}"));
    let mut prober = Prober::new(Rc::new(spec), mode);
    if matrix {
        prober.hooks = source::layer_hooks(layer.as_deref().map(std::path::Path::new)).unwrap_or_else(|e| panic!("layer: {e}"));
    }
    let items: Vec<Value> = doc[if cmd == "replay" { "scenarios" } else { "weapons" }].as_array().cloned().unwrap_or_default();
    let lo: usize = pos.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let hi: usize = pos.get(3).and_then(|s| s.parse::<usize>().ok()).map(|c| (lo + c).min(items.len())).unwrap_or(items.len());
    let dt = doc["dt"].as_f64().unwrap_or(mh_parity::DT);
    let mut f = std::io::BufWriter::new(std::fs::File::create(&pos[1]).expect("create out"));
    write!(f, "{{").unwrap();
    let mut n = 0;
    for it in items.iter().take(hi).skip(lo) {
        let (key, val) = if cmd == "replay" {
            let rows = prober.replay(it, dt);
            (it["id"].as_str().unwrap_or("").to_string(), Value::Array(rows.iter().map(|r| r.to_json()).collect()))
        } else {
            let wp = it.as_str().unwrap_or("").to_string();
            let lateness = doc["lateness"][&wp].as_f64().unwrap_or(0.5);
            let parriers: Vec<String> = doc["parriers"][&wp]
                .as_array()
                .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
                .unwrap_or_default();
            let v = prober.probe_weapon(&wp, lateness, &parriers);
            (wp, v)
        };
        write!(f, "{}{}:{}", if n > 0 { "," } else { "" }, Value::String(key), val).unwrap();
        f.flush().unwrap();
        n += 1;
    }
    write!(f, "}}").unwrap();
    f.flush().unwrap();
    println!("PARITY_DONE {n}");
}

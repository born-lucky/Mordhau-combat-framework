//! mh-spec-src: writes godot/data_gen/spec_src/character_exe.json, the horse / projectile / ladder-mover record dump the
//! spec populate reads (scripts/sheets_character_exe.py), with mh-character's own loaders over extract/json
//! (mh_host::exe_records::dump). Run by scripts/sheets_refresh.sh through scripts/cargo.sh.
//!   mh-spec-src [--repo <dir>]   (default: $MORDHAU_REPO, else the repo this crate is in)
use std::path::PathBuf;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let repo = a
        .iter()
        .position(|x| x == "--repo")
        .and_then(|i| a.get(i + 1))
        .map(PathBuf::from)
        .or_else(|| std::env::var("MORDHAU_REPO").ok().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."));
    let root = repo.join("extract/json");
    let v = match mh_host::exe_records::dump(&root) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("mh-spec-src: {e}");
            std::process::exit(1);
        }
    };
    let out = repo.join("godot/data_gen/spec_src/character_exe.json");
    let text = serde_json::to_string_pretty(&v).unwrap() + "\n";
    if std::fs::read_to_string(&out).ok().as_deref() == Some(text.as_str()) {
        println!("mh-spec-src: {} up to date", out.display());
        return;
    }
    std::fs::write(&out, text).unwrap_or_else(|e| panic!("{}: {e}", out.display()));
    println!("mh-spec-src: wrote {} ({} projectile classes)", out.display(), v["projectiles"].as_array().map_or(0, |a| a.len()));
}

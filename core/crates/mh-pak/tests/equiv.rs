//! Port of godot/tests/test_pak_equiv.gd: the pak reader equals extract/json field by field over the packages the
//! game's readers load (weapons and their parents, motions, character, bot profiles, AI, mode Blueprints, wearables'
//! bases, HUD widgets, map metadata, the UMA skeleton, the maps the modes load). MORDHAU_PAK_EQUIV=full also runs every
//! package under Blueprints/ and UI/ (the docs/PAK_FORMAT.md numbers). Differences allowed: CUE4Parse's NetQuantize
//! misread (pak side right), name trims, imports CUE4Parse left unresolved; anything else fails.

use mh_pak::{equiv, Reader, Vfs};
use std::sync::Arc;
use std::time::Instant;

fn setup() -> Option<(Reader, std::path::PathBuf)> {
    let v = match Vfs::mount_default() {
        Ok(v) => Arc::new(v),
        Err(e) => {
            eprintln!("SKIP: {e}");
            return None;
        }
    };
    let root = equiv::extract_root();
    if !root.join("json").exists() {
        eprintln!("SKIP: no extract/json under {}", root.display());
        return None;
    }
    Some((Reader::new(v), root))
}

fn report(pe: &equiv::Equiv, secs: f64) -> usize {
    let (known, real) = pe.split_misreads();
    println!("  {secs:.1} s");
    println!("  pak vs json: {}; {} of them CUE4Parse NetQuantize misreads", pe.summary(), known.len());
    println!("  undecoded values by export type: {:?}", pe.unsupported_types);
    println!("  undecoded values by reason: {:?}", pe.unsupported_reasons);
    for m in real.iter().take(40) {
        println!("    MISMATCH {} {}: pak={} json={}", m.pkg, m.path, m.pak, m.json);
    }
    real.len()
}

#[test]
fn pak_equals_json_for_used_packages() {
    let Some((rd, root)) = setup() else { return };
    let t = Instant::now();
    let pe = equiv::run(&rd, &root, equiv::SETS, equiv::SINGLES);
    let real = report(&pe, t.elapsed().as_secs_f64());
    assert_eq!(real, 0, "mismatches beyond the documented CUE4Parse misreads");
    let bad = equiv::disallowed_unsupported(&pe);
    assert!(bad.is_empty(), "values the pak reader cannot decode in exports the game uses: {bad:?}");
    assert!(pe.packages >= 700, "only {} packages compared", pe.packages);
}

#[test]
fn pak_equals_json_full() {
    if std::env::var("MORDHAU_PAK_EQUIV").as_deref() != Ok("full") {
        println!("  (set MORDHAU_PAK_EQUIV=full for every package under Blueprints/ and UI/)");
        return;
    }
    let Some((rd, root)) = setup() else { return };
    let t = Instant::now();
    let pe = equiv::run(&rd, &root, equiv::FULL, &[]);
    let real = report(&pe, t.elapsed().as_secs_f64());
    assert_eq!(real, 0, "mismatches beyond the documented CUE4Parse misreads");
}

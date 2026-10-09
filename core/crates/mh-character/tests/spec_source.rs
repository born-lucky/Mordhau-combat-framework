//! SpecCharacter (src/spec_source.rs, feature `spec`) == RecordsJson (the reference's record dump,
//! core/tests/golden/character/records.json) for every record value, compared as f32: the spec holds the exe's
//! binary32, the dump the reference's f64. Run: sh scripts/cargo.sh test -p mh-character --features spec --test spec_source
//! Skipped (with a message) when data_gen/spec or the dump is absent (local Triternion data).
use mh_character::spec_source::SpecCharacter;
use mh_character::{CharacterSource, RecordsJson};
use std::path::PathBuf;

fn root() -> PathBuf {
    std::env::var("MORDHAU_REPO").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../.."))
}

/// two Debug renderings with every float printed as its f32 (so f64-vs-f32 widening differences vanish)
fn f32_debug(s: &str) -> String {
    let mut out = String::new();
    let b: Vec<char> = s.chars().collect();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        if c.is_ascii_digit() || (c == '-' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit())) {
            let start = i;
            i += 1;
            while i < b.len() && (b[i].is_ascii_digit() || b[i] == '.' || b[i] == 'e' || (b[i] == '-' && b[i - 1] == 'e')) {
                i += 1;
            }
            let t: String = b[start..i].iter().collect();
            match t.parse::<f64>() {
                Ok(x) if t.contains('.') || t.contains('e') => out.push_str(&format!("{}", x as f32)),
                _ => out.push_str(&t),
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

#[test]
fn spec_records_equal_the_dump_as_f32() {
    let r = root();
    let spec_dir = std::env::var("MORDHAU_SPEC_DIR").map(PathBuf::from).unwrap_or_else(|_| r.join("data_gen/spec"));
    let dump = r.join("core/tests/golden/character/records.json");
    if !spec_dir.join("index.json").exists() || !dump.exists() {
        eprintln!("SKIP: need {} and {}", spec_dir.display(), dump.display());
        return;
    }
    let spec = mh_spec::Spec::load(&spec_dir, false).expect("spec");
    let a = SpecCharacter(&spec).load().expect("spec records");
    let b = RecordsJson(&std::fs::read_to_string(&dump).unwrap()).load().expect("dump records");
    let (sa, sb) = (f32_debug(&format!("{a:#?}")), f32_debug(&format!("{b:#?}")));
    if sa != sb {
        for (x, y) in sa.lines().zip(sb.lines()) {
            if x != y {
                panic!("spec {x:?} != dump {y:?}");
            }
        }
        panic!("record shapes differ");
    }
    eprintln!("SpecCharacter == RecordsJson (f32): movement, move_extra, character, physics, {} curve(s)", a.curves.len());
}

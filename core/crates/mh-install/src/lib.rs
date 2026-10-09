//! A preflight, not an importer or license/ownership verifier. Original files stay in place.
//! No game launch, download, copy, or generated-data fallback is performed here.
use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::{Component, Path, PathBuf};

pub const EXE_SHA1: &str = "dfe6f4fcb8e8a4603198025e13438b10062b0c4a";
pub const BUILD: &str = "702625635";
pub const DEFAULT_STEAM_INSTALL: &str = "C:/Program Files (x86)/Steam/steamapps/common/Mordhau";
const EXE: &str = "Mordhau/Binaries/Win64/Mordhau-Win64-Shipping.exe";
const PAK: &str = "Mordhau/Content/Paks/pakchunk0-WindowsClient.pak";
const DLL_DIR: &str = "Engine/Binaries/ThirdParty/PhysX3/Win64/VS2015";
const DLLS: [&str; 5] = ["PxFoundation_x64.dll", "PxPvdSDK_x64.dll", "PhysX3Common_x64.dll", "PhysX3_x64.dll", "PhysX3Cooking_x64.dll"];
const MOTION_HEADERS: [&str; 13] = ["UAttackMotion", "UBlockedMotion", "UCouchedAttackMotion", "UDisarmedMotion", "UFeintedMotion",
    "UFlinchMotion", "UIdleMotion", "UKickMotion", "UMordhauMotion", "UParryMotion", "UStabMotion", "UStrikeMotion", "UStunMotion"];
type Result<T> = std::result::Result<T, String>;

#[derive(Debug, Clone)]
pub struct Install {
    root: PathBuf,
    exe: PathBuf,
    exe_sha1: String,
    pak: PathBuf,
    physics_dlls: Vec<PathBuf>,
}

fn directory(path: &Path, what: &str) -> Result<PathBuf> {
    let p = fs::canonicalize(path).map_err(|e| format!("{what} {}: {e}", path.display()))?;
    if !p.is_dir() { return Err(format!("{what} is not a directory: {}", p.display())); }
    Ok(p)
}

fn local_file(root: &Path, relative: &str) -> Result<PathBuf> {
    let rel = Path::new(relative);
    if rel.is_absolute() || relative.contains(':') || relative.contains('\\') ||
        rel.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(format!("Invalid relative input path: {relative}"));
    }
    let p = fs::canonicalize(root.join(rel)).map_err(|e| format!("Missing required local input {relative}: {e}"))?;
    if !p.starts_with(root) { return Err(format!("Required input escapes selected directory: {relative}")); }
    let m = fs::metadata(&p).map_err(|e| e.to_string())?;
    if !m.is_file() || m.len() == 0 { return Err(format!("Required input is not a nonempty file: {}", p.display())); }
    Ok(p)
}

pub fn sha1(path: &Path) -> Result<String> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let mut hash = sha1_smol::Sha1::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 { break; }
        hash.update(&buf[..n]);
    }
    Ok(hash.digest().to_string())
}

/// Check file-backed DOS/PE headers, architecture, and PE32+ magic before reading the hash.
fn amd64_pe(path: &Path) -> Result<()> {
    let mut f = File::open(path).map_err(|e| e.to_string())?;
    let size = f.metadata().map_err(|e| e.to_string())?.len();
    let mut dos = [0u8; 64];
    f.read_exact(&mut dos).map_err(|e| format!("{}: truncated DOS header: {e}", path.display()))?;
    if dos[..2] != *b"MZ" { return Err(format!("{}: missing MZ header", path.display())); }
    let pe = u32::from_le_bytes(dos[60..64].try_into().unwrap()) as u64;
    if pe < 64 || pe.checked_add(26).filter(|end| *end <= size).is_none() {
        return Err(format!("{}: PE header lies outside the file", path.display()));
    }
    f.seek(SeekFrom::Start(pe)).map_err(|e| e.to_string())?;
    let mut nt = [0u8; 26];
    f.read_exact(&mut nt).map_err(|e| e.to_string())?;
    if nt[..4] != *b"PE\0\0" { return Err(format!("{}: missing PE signature", path.display())); }
    if u16::from_le_bytes([nt[4], nt[5]]) != 0x8664 || u16::from_le_bytes([nt[24], nt[25]]) != 0x20b {
        return Err(format!("{}: supported original files must be AMD64 PE32+ (64 bit)", path.display()));
    }
    let optional_size = u16::from_le_bytes([nt[20], nt[21]]) as u64;
    if optional_size < 112 || pe.checked_add(24 + optional_size).filter(|end| *end <= size).is_none() {
        return Err(format!("{}: truncated PE32+ optional header", path.display()));
    }
    Ok(())
}

impl Install {
    pub fn root(&self) -> &Path { &self.root }
    pub fn exe(&self) -> &Path { &self.exe }
    pub fn exe_sha1(&self) -> &str { &self.exe_sha1 }
    pub fn pak(&self) -> &Path { &self.pak }
    pub fn physics_dlls(&self) -> &[PathBuf] { &self.physics_dlls }
    pub fn discover() -> Result<Self> {
        let path = match std::env::var_os("MORDHAU_DIR") {
            Some(p) if p.is_empty() => return Err("MORDHAU_DIR is empty; select your own installed game directory".into()),
            Some(p) => PathBuf::from(p),
            None => PathBuf::from(DEFAULT_STEAM_INSTALL),
        };
        Self::verify(&path)
    }

    /// Launch consumes a prepared cache plus local packages/libraries. The EXE is
    /// an import input, not an ownership token that must be re-read at every launch.
    /// exe_sha1 here identifies the supported import format, not an observed file.
    pub fn runtime_files(root: &Path) -> Result<Self> {
        let root = directory(root, "Local Mordhau runtime files")?;
        let (pak, physics_dlls) = runtime_payloads(&root)?;
        Ok(Self { exe: root.join(EXE), exe_sha1: EXE_SHA1.into(), root, pak, physics_dlls })
    }

    pub fn discover_runtime() -> Result<Self> {
        let root = match std::env::var_os("MORDHAU_DIR") {
            Some(p) if p.is_empty() => return Err("MORDHAU_DIR is empty".into()),
            Some(p) => PathBuf::from(p),
            None => PathBuf::from(DEFAULT_STEAM_INSTALL),
        };
        Self::runtime_files(&root)
    }

    pub fn verify(root: &Path) -> Result<Self> { verify_install(root, EXE_SHA1) }
}

// The expected hash is private, and production has only the fixed original hash entry point.
// Unit fixtures invoke this helper with a synthetic hash; there is no CLI/env bypass.
fn verify_install(root: &Path, expected_sha1: &str) -> Result<Install> {
    let root = directory(root, "Original Mordhau installation")?;
    let exe = local_file(&root, EXE)?;
    amd64_pe(&exe)?;
    let hash = sha1(&exe)?;
    if hash != expected_sha1 {
        return Err(format!("Unsupported original EXE: SHA1 {hash}; expected {expected_sha1} (build {BUILD})"));
    }
    let (pak, physics_dlls) = runtime_payloads(&root)?;
    Ok(Install { root, exe, exe_sha1: hash, pak, physics_dlls })
}

fn runtime_payloads(root: &Path) -> Result<(PathBuf, Vec<PathBuf>)> {
    for marker in ["version.txt", "installedversion", "installedversion.txt"] {
        if root.join(marker).exists() {
            let file = local_file(&root, marker)?;
            let value = fs::read_to_string(file).map_err(|e| e.to_string())?;
            if value.trim() != BUILD { return Err(format!("Unsupported original install version in {marker}: expected {BUILD}")); }
        }
    }
    let pak = local_file(&root, PAK)?;
    let mut physics_dlls = Vec::new();
    for dll in DLLS {
        let p = local_file(&root, &format!("{DLL_DIR}/{dll}"))?;
        amd64_pe(&p)?;
        physics_dlls.push(p);
    }
    Ok((pak, physics_dlls))
}

#[derive(Debug, Clone)]
pub struct RuntimeInputs {
    pub install: Install,
    pub local_data: PathBuf,
    pub spec: PathBuf,
    pub extract: PathBuf,
}

impl RuntimeInputs {
    pub fn discover() -> Result<Self> {
        let install = Install::discover_runtime()?;
        Self::local_from_environment(install)
    }

    pub fn local_from_environment(install: Install) -> Result<Self> {
        let local = match std::env::var_os("MORDHAU_LOCAL_DATA") {
            Some(p) if p.is_empty() => return Err("MORDHAU_LOCAL_DATA is empty".into()),
            Some(p) => PathBuf::from(p),
            None => PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("Set MORDHAU_LOCAL_DATA; LOCALAPPDATA is unavailable")?)
                .join("mordhau-rewrite-rust/v1").join(&install.exe_sha1),
        };
        Self::verify_local(install, &local)
    }

    pub fn verify_local(install: Install, local: &Path) -> Result<Self> {
        let local_data = directory(local, "Local import cache (complete setup is required)")?;
        let required = |name: &str| local_file(&local_data, name)
            .map_err(|e| format!("Local preparation is incomplete. {e}"));
        let spec = directory(&local_data.join("data_gen/spec"), "Local generated spec matrix")?;
        let index_path = required("data_gen/spec/index.json")?;
        let read_json = |file: &Path| -> Result<serde_json::Value> {
            let txt = fs::read_to_string(file).map_err(|e| e.to_string())?;
            serde_json::from_str(&txt).map_err(|e| format!("{}: invalid generated JSON: {e}", file.display()))
        };
        let index = read_json(&index_path)?;
        if index["format"] != "mordhau-spec/1" { return Err("Unsupported local matrix format; release blocked".into()); }
        let types = index["entity_types"].as_object().filter(|v| !v.is_empty()).ok_or("Local matrix index has no entity types; release blocked")?;
        for kind in ["weapon", "attack", "motion", "character", "movement", "stat", "camera", "physics", "constant", "bot", "bt", "wearable", "equipment", "setting"] {
            if !types.contains_key(kind) { return Err(format!("Local matrix lacks required entity type {kind}; release blocked")); }
        }
        for base in ["fields.json", "rules.json", "sources.json"] {
            let f = required(&format!("data_gen/spec/{base}"))?;
            if !read_json(&f)?.as_object().is_some_and(|o| !o.is_empty()) { return Err(format!("Local matrix {base} is not a nonempty object; release blocked")); }
        }
        let mut entities = 0u64;
        for info in types.values() {
            let relative = info["file"].as_str().ok_or("Local matrix entity index lacks file")?;
            let file = required(&format!("data_gen/spec/{relative}"))?;
            let value = read_json(&file)?;
            let object = value.as_object().ok_or("Local matrix entity file is not an object")?;
            if info["entities"].as_u64() != Some(object.len() as u64) { return Err(format!("Local matrix count mismatch: {relative}")); }
            entities += object.len() as u64;
        }
        if entities == 0 || index["counts"]["entities"].as_u64() != Some(entities) { return Err("Local matrix total entity count mismatch; release blocked".into()); }
        let constants = required("extract/native/rdata.tsv")?;
        let text = fs::read_to_string(constants).map_err(|e| e.to_string())?;
        for va in ["14406a414", "144317d48", "144317ba0", "144317b90", "144317bb0", "144317bd0", "144317bc0", "144317bd8"] {
            if !text.lines().skip(1).any(|l| { let c:Vec<_>=l.split('\t').collect(); c.len()>=9 && c[0].eq_ignore_ascii_case(va) }) {
                return Err(format!("Local native cache lacks required record {va}; release blocked"));
            }
        }
        for name in MOTION_HEADERS { required(&format!("extract/native/types/{name}.h"))?; }
        for file in ["shadow_capsules.json", "state/physics/mh_physx.dll"] { required(file)?; }
        let extract = directory(&local_data.join("extract"), "Local original import directory")?;
        Ok(Self { install, local_data, spec, extract })
    }

    /// Call only before threads/Bevy are initialized. Override developer paths so none can leak into v1.
    pub fn apply_environment(&self) {
        std::env::set_var("MORDHAU_DIR", &self.install.root);
        std::env::set_var("MORDHAU_LOCAL_DATA", &self.local_data);
        std::env::set_var("MORDHAU_REPO", &self.local_data);
        std::env::set_var("MORDHAU_SPEC_DIR", &self.spec);
        std::env::set_var("MORDHAU_EXTRACT", &self.extract);
        std::env::set_var("MH_SHADER_DIR", self.local_data.join("data_gen/shaders/particles"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let p = std::env::temp_dir().join(format!("mh-install-tests-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
            fs::create_dir(&p).unwrap(); Self(p)
        }
        fn put(&self, rel: &str, bytes: &[u8]) {
            let p = self.0.join(rel); fs::create_dir_all(p.parent().unwrap()).unwrap(); fs::write(p, bytes).unwrap();
        }
    }
    impl Drop for Temp { fn drop(&mut self) { let _ = fs::remove_dir_all(&self.0); } }
    fn pe() -> Vec<u8> {
        let mut p = vec![0; 512]; p[..2].copy_from_slice(b"MZ"); p[60..64].copy_from_slice(&128u32.to_le_bytes());
        p[128..132].copy_from_slice(b"PE\0\0"); p[132..134].copy_from_slice(&0x8664u16.to_le_bytes());
        p[148..150].copy_from_slice(&240u16.to_le_bytes()); p[152..154].copy_from_slice(&0x20bu16.to_le_bytes()); p
    }
    fn install() -> (Temp, String) {
        let t = Temp::new(); t.put(EXE, &pe()); t.put(PAK, b"synthetic unit fixture only"); t.put("version.txt", BUILD.as_bytes());
        for name in DLLS { t.put(&format!("{DLL_DIR}/{name}"), &pe()); }
        let hash = sha1(&t.0.join(EXE)).unwrap(); (t, hash)
    }
    #[test] fn rejects_missing_install_and_fake_original_hash() {
        let t = Temp::new(); assert!(Install::verify(&t.0).unwrap_err().contains("Missing required"));
        let (t, _) = install(); assert!(Install::verify(&t.0).unwrap_err().contains("Unsupported original EXE"));
    }
    #[test] fn prepared_runtime_does_not_require_exe_but_import_still_does() {
        let (t, _) = install();
        fs::remove_file(t.0.join(EXE)).unwrap();
        let files = Install::runtime_files(&t.0).unwrap();
        assert_eq!(files.exe_sha1(), EXE_SHA1);
        assert_eq!(files.physics_dlls().len(), 5);
        assert!(Install::verify(&t.0).unwrap_err().contains("Shipping.exe"));
        fs::remove_file(t.0.join(PAK)).unwrap();
        assert!(Install::runtime_files(&t.0).unwrap_err().contains("pakchunk0"));
    }
    #[test] fn synthetic_policy_fixture_checks_complete_install_and_missing_files() {
        let (t, hash) = install(); let i = verify_install(&t.0, &hash).unwrap(); assert_eq!(i.physics_dlls.len(), 5);
        fs::remove_file(t.0.join(PAK)).unwrap(); assert!(verify_install(&t.0, &hash).unwrap_err().contains("pakchunk0"));
        t.put(PAK, b"synthetic only"); fs::remove_file(t.0.join(DLL_DIR).join(DLLS[3])).unwrap();
        assert!(verify_install(&t.0, &hash).unwrap_err().contains(DLLS[3]));
    }
    #[test] fn rejects_malformed_or_wrong_architecture_headers_before_hash() {
        let (t, hash) = install();
        for bad in [vec![0; 64], b"MZ".to_vec(), { let mut p=pe(); p[60..64].copy_from_slice(&u32::MAX.to_le_bytes()); p },
            { let mut p=pe(); p[132..134].copy_from_slice(&0x14cu16.to_le_bytes()); p },
            { let mut p=pe(); p[152..154].copy_from_slice(&0x10bu16.to_le_bytes()); p }] {
            t.put(EXE, &bad); assert!(!verify_install(&t.0, &hash).unwrap_err().contains("Unsupported original EXE"));
        }
    }
    #[test] fn rejects_wrong_optional_install_version_and_dll_header() {
        let (t, hash) = install(); t.put("version.txt", b"different build");
        assert!(verify_install(&t.0, &hash).unwrap_err().contains("install version"));
        t.put("version.txt", BUILD.as_bytes()); t.put(&format!("{DLL_DIR}/{}", DLLS[0]), b"not PE");
        assert!(verify_install(&t.0, &hash).unwrap_err().contains("DOS header"));
    }
    #[test] fn incomplete_local_imports_block_even_valid_policy_install() {
        let (t, hash)=install(); let i=verify_install(&t.0,&hash).unwrap(); let local=Temp::new();
        assert!(RuntimeInputs::verify_local(i,&local.0).unwrap_err().contains("spec matrix"));
    }
    #[test] fn indexed_paths_cannot_escape_local_cache() {
        let t=Temp::new(); for name in ["../outside", "C:/outside", "/outside", "a\\b"] {
            assert!(local_file(&fs::canonicalize(&t.0).unwrap(),name).unwrap_err().contains("Invalid relative"));
        }
    }
    #[test] fn malformed_and_indexed_missing_local_records_fail_closed() {
        let (t,hash)=install(); let i=verify_install(&t.0,&hash).unwrap(); let local=Temp::new();
        local.put("data_gen/spec/index.json",b"not json");
        assert!(RuntimeInputs::verify_local(i.clone(),&local.0).unwrap_err().contains("invalid generated JSON"));
        let kinds=["weapon", "attack", "motion", "character", "movement", "stat", "camera", "physics", "constant", "bot", "bt", "wearable", "equipment", "setting"];
        let entries:serde_json::Map<String,serde_json::Value>=kinds.iter().map(|k|(k.to_string(),serde_json::json!({"file":format!("entities/{k}.json"),"entities":1}))).collect();
        let index=serde_json::json!({"format":"mordhau-spec/1","entity_types":entries,"counts":{"entities":14}});
        local.put("data_gen/spec/index.json",index.to_string().as_bytes());
        for name in ["fields.json","rules.json","sources.json"] {local.put(&format!("data_gen/spec/{name}"),b"{\"synthetic_only\":{}}");}
        assert!(RuntimeInputs::verify_local(i.clone(),&local.0).unwrap_err().contains("entities/"));
        for k in kinds {local.put(&format!("data_gen/spec/entities/{k}.json"),b"{\"synthetic_only\":{}}");}
        assert!(RuntimeInputs::verify_local(i,&local.0).unwrap_err().contains("rdata.tsv"));
    }
}

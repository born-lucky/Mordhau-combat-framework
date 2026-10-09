//! The mounted file system of every .pak in the user's Mordhau install, read in place (read-only; nothing is ever
//! written under the game folder). Port of `ue_pak_vfs.gd` `UePakVfs`. Paths are the mounted paths without the
//! "../../../" root, as in extract/manifest.tsv: "Mordhau/Content/Mordhau/Blueprints/.../BP_X.uasset". Lookups ignore
//! case like UE's FPakPlatformFile (Windows); the stored spelling is kept for package names.
//!
//! `Vfs` is immutable after `mount` and `Send + Sync`: share one `Arc<Vfs>` between threads.

use crate::pak::{Bytes, Entry, Pak, PakError};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

/// mount points are relative to Engine/Binaries/Win64 (FPakFile mount point)
pub const ROOT_PREFIX: &str = "../../../";

/// Default Steam install folder (the same default as build.py:10 GAME and UePakVfs.game_dir)
pub const STEAM_DEFAULT: &str = "C:/Program Files (x86)/Steam/steamapps/common/Mordhau";

/// The game folder: $MORDHAU_DIR, else Steam's default folder (UePakVfs.game_dir; the Godot project setting
/// mordhau/game_dir has no Rust counterpart)
pub fn game_dir() -> PathBuf {
    match std::env::var("MORDHAU_DIR") {
        Ok(d) if !d.is_empty() => PathBuf::from(d),
        _ => PathBuf::from(STEAM_DEFAULT),
    }
}

/// The install folder <game>\Mordhau\Content\Paks (an install path, not a package: the paks hold the packages)
pub fn paks_dir(game: &Path) -> PathBuf {
    game.join("Mordhau").join("Content").join("Paks")
}

struct File {
    spelling: String,
    slot: u16,
    loc: i32,
}

pub struct Vfs {
    pub paks: Vec<Pak>,
    files: Vec<File>,
    /// lower-case path -> index into `files`
    index: HashMap<String, u32>,
    /// paks that failed to open, duplicate paths
    pub errors: Vec<String>,
}

impl Vfs {
    /// Mount every *.pak of the install at `game_dir()`
    pub fn mount_default() -> Result<Vfs, PakError> {
        Vfs::mount(&paks_dir(&game_dir()))
    }

    /// Mount every *.pak in `dir` (sorted by name). Err when the folder has no readable pak.
    pub fn mount(dir: &Path) -> Result<Vfs, PakError> {
        let rd = std::fs::read_dir(dir).map_err(|e| PakError(format!("no pak folder {}: {e}", dir.display())))?;
        let mut names: Vec<String> =
            rd.filter_map(|e| e.ok()).map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".pak")).collect();
        names.sort();
        let mut v = Vfs { paks: Vec::new(), files: Vec::new(), index: HashMap::new(), errors: Vec::new() };
        for n in names {
            let mut raw = Vec::new();
            let pk = match Pak::open(&dir.join(&n), &mut raw) {
                Ok(p) => p,
                Err(e) => {
                    v.errors.push(format!("{n}: {e}"));
                    continue;
                }
            };
            let slot = v.paks.len() as u16;
            v.paks.push(pk);
            for (k, loc) in raw {
                let pth = k.strip_prefix(ROOT_PREFIX).unwrap_or(&k).to_string();
                let low = pth.to_lowercase();
                let f = File { spelling: pth, slot, loc };
                match v.index.get(&low) {
                    Some(&i) => {
                        // UE resolves duplicates by pak read order (_P patch paks win); Mordhau 702625635 has none
                        v.errors.push(format!("duplicate {}", f.spelling));
                        v.files[i as usize] = f;
                    }
                    None => {
                        v.index.insert(low, v.files.len() as u32);
                        v.files.push(f);
                    }
                }
            }
        }
        if v.paks.is_empty() {
            return Err(PakError(format!("no readable pak in {}: {}", dir.display(), v.errors.join(", "))));
        }
        Ok(v)
    }

    fn find(&self, pth: &str) -> Option<&File> {
        self.index.get(&pth.to_lowercase()).map(|&i| &self.files[i as usize])
    }
    pub fn has(&self, pth: &str) -> bool {
        self.find(pth).is_some()
    }
    /// The path as stored in the pak (package names keep it)
    pub fn spelling(&self, pth: &str) -> String {
        self.find(pth).map_or_else(|| pth.to_string(), |f| f.spelling.clone())
    }
    pub fn file_count(&self) -> usize {
        self.files.len()
    }
    /// Every mounted path (stored spelling), in mount order
    pub fn list(&self) -> impl Iterator<Item = &str> {
        self.files.iter().map(|f| f.spelling.as_str())
    }
    /// Every mounted entry decoded: {"method <name>": count, "encrypted": count}
    pub fn census(&self) -> BTreeMap<String, usize> {
        let mut out = BTreeMap::new();
        out.insert("encrypted".to_string(), 0);
        for f in &self.files {
            let pk = &self.paks[f.slot as usize];
            let e = pk.entry(f.loc);
            let k = format!("method {}", pk.compression.get(e.method as usize).cloned().unwrap_or_else(|| e.method.to_string()));
            *out.entry(k).or_insert(0) += 1;
            *out.get_mut("encrypted").unwrap() += e.encrypted as usize;
        }
        out
    }
    /// Entry info of a path and the file name of its pak
    pub fn entry(&self, pth: &str) -> Option<(Entry, String)> {
        let f = self.find(pth)?;
        let pk = &self.paks[f.slot as usize];
        Some((pk.entry(f.loc), pk.file_name()))
    }
    /// Bytes of a file (a view into the pak mapping); None when missing or unreadable
    pub fn read(&self, pth: &str, verify: bool) -> Option<Bytes> {
        self.try_read(pth, verify).ok()
    }
    pub fn try_read(&self, pth: &str, verify: bool) -> Result<Bytes, PakError> {
        let f = self.find(pth).ok_or_else(|| PakError(format!("not in the paks: {pth}")))?;
        self.paks[f.slot as usize].read(f.loc, verify)
    }
    /// `size` bytes at `start` of a file (Pak::read_range); None when missing or out of range
    pub fn read_range(&self, pth: &str, start: u64, size: u64) -> Option<Bytes> {
        let f = self.find(pth)?;
        self.paks[f.slot as usize].read_range(f.loc, start, size).ok()
    }
}

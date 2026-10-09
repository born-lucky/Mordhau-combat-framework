//! paths.rs - where the runtime reads data from (mirrors godot/project.godot [mordhau] data_path / game_dir).
//! R1 reads extract/ (the Godot `json` backend inputs); R4 adds the user's Mordhau install through mh-pak.

use bevy::prelude::Resource;
use std::path::{Path, PathBuf};

#[derive(Resource, Clone, Debug)]
#[allow(dead_code)] // extract: R4 pak fallback
pub struct Paths {
    /// repo root (has state/rust_brief.md)
    pub repo: PathBuf,
    /// extract/ ($MORDHAU_EXTRACT overrides)
    pub extract: PathBuf,
    /// extract/gltf: glb meshes, PNG textures, material JSON (= godot/data). The Bevy asset root.
    pub data: PathBuf,
    /// extract/json: package JSON (`mdx json`)
    pub json: PathBuf,
}

fn find_repo(start: &Path) -> Option<PathBuf> {
    let mut d = Some(start);
    while let Some(p) = d {
        if p.join("state").join("rust_brief.md").is_file() || p.join("core").join("Cargo.toml").is_file() {
            return Some(p.to_path_buf());
        }
        d = p.parent();
    }
    None
}

impl Paths {
    pub fn discover() -> Paths {
        let repo = std::env::var_os("MORDHAU_REPO")
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok().and_then(|c| find_repo(&c)))
            .or_else(|| std::env::current_exe().ok().and_then(|e| e.parent().and_then(find_repo)))
            .unwrap_or_else(|| PathBuf::from("."));
        let extract = std::env::var_os("MORDHAU_EXTRACT").map(PathBuf::from).unwrap_or_else(|| repo.join("extract"));
        Paths { data: extract.join("gltf"), json: extract.join("json"), extract, repo }
    }
}

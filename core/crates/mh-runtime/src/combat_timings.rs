//! Rewrite development controls for explicit original-record timing edits; no native CVar claim.
use bevy::prelude::*;
use std::path::{Path, PathBuf};
use std::io::Write;

#[derive(Resource, Debug, serde::Serialize)]
pub struct Controls {
    pub path: PathBuf,
    pub defaults_path: PathBuf,
    pub message: String,
    #[serde(skip)]
    seen_catalog: Option<u64>,
}
impl Default for Controls {
    fn default() -> Self {
        let root = crate::paths::Paths::discover().repo.join("state/mods");
        Self { path: std::env::var_os("MORDHAU_TIMINGS_FILE").map(PathBuf::from)
            .unwrap_or_else(|| root.join("combat-timings.json")),
            defaults_path: root.join("combat-timings.defaults.json"), message: "Stock timings".into(), seen_catalog: None }
    }
}

pub struct CombatTimingsPlugin;
impl Plugin for CombatTimingsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Controls>().add_systems(Update, autoload.after(crate::sim::tick_sim_frame));
    }
}

fn autoload(world: &mut World) {
    let status = world.get_non_send_resource::<crate::sim::Sim>().map(|s|s.0.combat_timing_status()).unwrap_or_default();
    if status["available"].as_bool() != Some(true) { world.resource_mut::<Controls>().seen_catalog = None; return; }
    if status["pending_idle"].as_bool() == Some(false) && world.resource::<Controls>().message.contains("waiting for idle") {
        world.resource_mut::<Controls>().message = format!("Combat timings applied (revision {})",status["revision"]);
    }
    let epoch = status["catalog_epoch"].as_u64().unwrap_or(0);
    if world.resource::<Controls>().seen_catalog == Some(epoch) { return; }
    world.resource_mut::<Controls>().seen_catalog = Some(epoch);
    if world.resource::<Controls>().path.is_file() { command(world,"reload"); }
}

fn write_json(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    if let Some(parent) = path.parent() { std::fs::create_dir_all(parent).map_err(|e| e.to_string())?; }
    let text = serde_json::to_string_pretty(value).map_err(|e| e.to_string())?;
    std::fs::write(path,text).map_err(|e| format!("{}: {e}",path.display()))
}

fn resolved_path(path: &Path) -> Result<String,String> {
    let p=std::fs::canonicalize(path).or_else(|_| std::path::absolute(path)).map_err(|e|e.to_string())?;
    // Windows paths are case-insensitive. Preserve exact existing symlink targets where available.
    Ok(p.to_string_lossy().to_lowercase())
}

fn create_sparse_patch(path: &Path) -> Result<(),String> {
    if let Some(parent)=path.parent() { std::fs::create_dir_all(parent).map_err(|e|e.to_string())?; }
    let text=serde_json::to_string_pretty(&mordhau_core::timing::TimingEdits::default()).map_err(|e|e.to_string())?;
    match std::fs::OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file)=>file.write_all(text.as_bytes()).map_err(|e|e.to_string()),
        Err(e) if e.kind()==std::io::ErrorKind::AlreadyExists=>Ok(()),
        Err(e)=>Err(e.to_string()),
    }
}

/// Called by the actual development menu and `debug rw.CombatTimings <action>` harness command.
pub fn command(world: &mut World, action: &str) -> bool {
    let action = action.trim().to_ascii_lowercase();
    if !matches!(action.as_str(),"export"|"reload"|"reset") { return false; }
    let result: Result<String,String> = (|| {
        let (path,defaults_path) = { let c=world.resource::<Controls>(); (c.path.clone(),c.defaults_path.clone()) };
        match action.as_str() {
            "export" => {
                if resolved_path(&path)? == resolved_path(&defaults_path)? {
                    return Err("the authored edit path aliases the stock export path; choose distinct files".into());
                }
                let export = world.non_send_resource::<crate::sim::Sim>().0.export_combat_timings()?;
                write_json(&defaults_path,&export)?;
                // Create a sparse starting patch only if absent; existing authored edits are never overwritten.
                create_sparse_patch(&path)?;
                Ok(format!("Exported stock catalog: {}",defaults_path.display()))
            }
            "reload" => {
                let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}",path.display()))?;
                let edits = mordhau_core::timing::parse(&text)?;
                let applied = world.non_send_resource_mut::<crate::sim::Sim>().0.apply_combat_timings(Some(&edits))?;
                Ok(if applied { "Combat timing edits applied".into() } else { "Validated timing edits waiting for idle".into() })
            }
            "reset" => {
                let applied = world.non_send_resource_mut::<crate::sim::Sim>().0.apply_combat_timings(None)?;
                Ok(if applied { "Stock timing values restored; edit file retained".into() }
                    else { "Stock timing reset waiting for idle".into() })
            }
            _ => unreachable!(),
        }
    })();
    let message = match result { Ok(m)=>m,Err(e)=>format!("Combat timings unchanged: {e}") };
    world.resource_mut::<Controls>().message = message.clone();
    bevy::log::info!("{message}");
    true
}

pub fn snapshot(world: &World) -> serde_json::Value {
    let controls = world.get_resource::<Controls>();
    let status = world.get_non_send_resource::<crate::sim::Sim>().map(|s|s.0.combat_timing_status());
    serde_json::json!({"controls":controls,"backend":status})
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let path = std::env::temp_dir().join(format!("mordhau-timing-controls-{}-{stamp}",std::process::id()));
            std::fs::create_dir_all(&path).unwrap();
            Self(path)
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) { let _ = std::fs::remove_dir_all(&self.0); }
    }

    #[test]
    fn export_rejects_alias_before_touching_authored_file() {
        let dir = Scratch::new();
        let path = dir.0.join("authored.json");
        let authored = br#"{"schema_version":1,"weapons":{"custom":{"values":{"stab_release_modifier":0}}}}"#;
        std::fs::write(&path,authored).unwrap();
        let mut world = World::new();
        world.insert_resource(Controls { path: path.clone(), defaults_path: dir.0.join(".").join("authored.json"),
            message: String::new(), seen_catalog: None });
        // No sim installed: the guard must precede export and any disk write.
        assert!(command(&mut world,"export"));
        assert!(world.resource::<Controls>().message.contains("aliases the stock export"));
        assert_eq!(std::fs::read(path).unwrap(),authored);
    }

    #[test]
    fn sparse_patch_creation_preserves_existing_authored_bytes() {
        let dir = Scratch::new();
        let path = dir.0.join("patch.json");
        create_sparse_patch(&path).unwrap();
        let empty = mordhau_core::timing::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(empty.weapons.is_empty() && empty.motions.is_empty() && empty.curves.is_empty());
        let authored = b"user-edited bytes, including unfinished work\r\n";
        std::fs::write(&path,authored).unwrap();
        create_sparse_patch(&path).unwrap();
        assert_eq!(std::fs::read(path).unwrap(),authored);
    }
}

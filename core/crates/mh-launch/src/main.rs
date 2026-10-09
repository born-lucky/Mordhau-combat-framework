//! MordhauRewrite.exe: the double-click launcher for the Rust rewrite. Placed at the repo root, it starts the current
//! release build (core/target/release/mordhau.exe) directly, the way scripts/runtime/play.ps1 does.
//! Close the game before rebuilding so Windows releases that executable for the linker.
//!
//!   MordhauRewrite.exe               -> the main menu (play.ps1 -Menu)
//!   MordhauRewrite.exe --test-level  -> the combat test level vs one bot (play.ps1 -TestLevel)
//!   MordhauRewrite.exe <game args>   -> passed through to mordhau.exe unchanged
//!
//! Errors (no release build yet, repo not found) show in a message box, since there is no console window.
#![windows_subsystem = "windows"]

use std::path::PathBuf;
use std::process::Command;

const DEFAULT_MAP: &str = "TestLevel";

/// the repo root: the launcher's own folder or one of its parents holding core/target/release
fn repo_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut d = exe.parent()?.to_path_buf();
    loop {
        if d.join("core").join("target").join("release").is_dir() {
            return Some(d);
        }
        if !d.pop() {
            return None;
        }
    }
}

fn args_for(user: &[String]) -> Vec<String> {
    match user.first().map(String::as_str) {
        None => vec!["--map".into(), DEFAULT_MAP.into(), "--menu".into()],
        Some("--test-level") => {
            let mut a = vec!["--map".into(), "TestLevel".into(), "--bots".into(), "1".into()];
            a.extend(user[1..].iter().cloned());
            a
        }
        Some(_) => user.to_vec(),
    }
}

fn fail(msg: &str) -> ! {
    // no console: show the error through the shell's message box
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-Command", &format!(
            "Add-Type -AssemblyName PresentationFramework; [System.Windows.MessageBox]::Show('{}', 'Mordhau Rewrite')",
            msg.replace('\'', "''"))])
        .status();
    std::process::exit(1)
}

fn main() {
    // Independent launcher gate; the runtime also repeats this preflight for direct entry.
    let local_inputs = match mh_install::RuntimeInputs::discover() {
        Ok(inputs) => inputs,
        Err(e) => fail(&format!("Private v1 launch blocked: {e}")),
    };
    local_inputs.apply_environment();
    let Some(root) = repo_root() else { fail("Can't find the project folder (core\\target\\release) next to this exe.") };
    let release = root.join("core").join("target").join("release").join("mordhau.exe");
    if !release.is_file() {
        fail(&format!("No v1 runtime at {}. This source snapshot has no playable release; see docs/RELEASE_BLOCKERS.md.", release.display()));
    }
    let user: Vec<String> = std::env::args().skip(1).collect();
    let started = Command::new(&release).args(args_for(&user)).current_dir(&root).spawn();
    if let Err(e) = started {
        fail(&format!("Couldn't start {}: {e}", release.display()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_the_main_menu_and_test_level_maps_to_play_ps1() {
        assert_eq!(args_for(&[]), ["--map", DEFAULT_MAP, "--menu"]);
        assert_eq!(args_for(&["--test-level".into(), "--profile".into(), "Knight".into()]),
            ["--map", "TestLevel", "--bots", "1", "--profile", "Knight"]);
        assert_eq!(args_for(&["--map".into(), "X".into()]), ["--map", "X"]);
    }
}

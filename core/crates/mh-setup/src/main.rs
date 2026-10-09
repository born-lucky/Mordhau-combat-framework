//! Setup shell and a scriptable check API. No original files or derived records are embedded.
#![cfg_attr(windows, windows_subsystem = "windows")]
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use serde_json::{json, Value};

const UI: &str = include_str!("setup.ps1");

#[derive(Default)]
struct Args { operation: String, game: Option<PathBuf>, root: Option<PathBuf>, preview: Option<PathBuf> }

fn args() -> Result<Args, String> {
    let mut a = Args::default();
    let mut input = std::env::args_os().skip(1);
    while let Some(v) = input.next() {
        match v.to_str() {
            Some("--check" | "--prepare" | "--launch") => {
                if !a.operation.is_empty() { return Err("Choose one operation".into()); }
                a.operation = v.to_string_lossy().into();
            }
            Some("--game-dir") => a.game = Some(input.next().ok_or("--game-dir needs a folder")?.into()),
            Some("--root") => a.root = Some(input.next().ok_or("--root needs a folder")?.into()),
            Some("--preview") => a.preview = Some(input.next().ok_or("--preview needs an output PNG")?.into()),
            Some("--json") => (),
            _ => return Err(format!("Unknown argument {}", v.to_string_lossy())),
        }
    }
    Ok(a)
}

fn framework_root() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let mut p = exe.parent().ok_or("Setup has no folder")?.to_path_buf();
    loop {
        if p.join("release-base.json").is_file() { return Ok(p); }
        if !p.pop() { break; }
    }
    Err("Cannot find framework files beside this setup executable".into())
}

fn runtime(root: &Path) -> Option<PathBuf> {
    [root.join("runtime/mordhau.exe"), root.join("core/target/release/mordhau.exe")]
        .into_iter().find(|p| p.is_file())
}

fn default_cache(install: &mh_install::Install) -> Result<PathBuf,String> {
    match std::env::var_os("MORDHAU_LOCAL_DATA") {
        Some(v) if !v.is_empty() => Ok(v.into()),
        Some(_) => Err("MORDHAU_LOCAL_DATA is empty".into()),
        None => Ok(PathBuf::from(std::env::var_os("LOCALAPPDATA").ok_or("LOCALAPPDATA unavailable")?)
            .join("mordhau-rewrite-rust/v1").join(install.exe_sha1())),
    }
}

fn verify(game: Option<&Path>) -> Result<mh_install::Install,String> {
    match game { Some(p) => mh_install::Install::verify(p), None => mh_install::Install::discover() }
}

fn check(game: Option<&Path>, root: &Path) -> Value {
    match verify(game) {
        Err(e) => json!({"installed":false,"ready":false,"error":e}),
        Ok(i) => {
            let cache = default_cache(&i);
            let inputs = match &cache { Ok(p) => mh_install::RuntimeInputs::verify_local(i.clone(),p), Err(e) => Err(e.clone()) };
            let executable = runtime(root);
            json!({"installed":true,"ready":inputs.is_ok() && executable.is_some(),
                "game_dir":i.root(),"exe_sha1":i.exe_sha1(),"cache":cache.ok(),
                "local_data_ready":inputs.is_ok(),"runtime_present":executable.is_some(),
                "error":inputs.err().or_else(|| if executable.is_none() { Some("The playable framework runtime has not been packaged yet".into()) } else {None})})
        }
    }
}

fn prepare(game: Option<&Path>, root: &Path) -> Result<Value,String> {
    let i = verify(game)?;
    let cache = default_cache(&i)?;
    let tool = root.join("tools/setup.py");
    if !tool.is_file() { return Err("Local import tools are not available in this source snapshot".into()); }
    let result = Command::new(std::env::var_os("MH_SETUP_PYTHON").unwrap_or_else(|| "python".into()))
        .arg(&tool).arg("--game-dir").arg(i.root()).arg("--cache-dir").arg(&cache)
        .current_dir(root).output().map_err(|e| format!("Cannot run local import tools; install Python 3.11 or set MH_SETUP_PYTHON: {e}"))?;
    if !result.status.success() {
        let out=String::from_utf8_lossy(&result.stdout); let err=String::from_utf8_lossy(&result.stderr);
        return Err(format!("Local setup has not completed: {out}\n{err}"));
    }
    Ok(check(Some(i.root()),root))
}

fn launch(game: Option<&Path>, root: &Path) -> Result<Value,String> {
    let i=verify(game)?;
    let cache=default_cache(&i)?;
    let data=mh_install::RuntimeInputs::verify_local(i,&cache)?;
    let exe=runtime(root).ok_or("Playable runtime is not available")?;
    let child=Command::new(&exe).arg("--map").arg("TestLevel").arg("--bots").arg("1")
        .env("MORDHAU_DIR",data.install.root()).env("MORDHAU_LOCAL_DATA",&data.local_data)
        .env("MORDHAU_REPO",&data.local_data).env("MORDHAU_SPEC_DIR",&data.spec)
        .env("MORDHAU_EXTRACT",&data.extract).current_dir(root).spawn().map_err(|e|e.to_string())?;
    Ok(json!({"launch_requested":true,"process_id":child.id(),
        "status":"A runtime process was created; gameplay readiness has not been confirmed"}))
}

fn run(a: Args) -> Result<Value,String> {
    let root=match a.root { Some(p)=>p, None=>framework_root()? };
    match a.operation.as_str() {
        "--check"=>Ok(check(a.game.as_deref(),&root)),
        "--prepare"=>prepare(a.game.as_deref(),&root),
        "--launch"=>launch(a.game.as_deref(),&root),
        ""=> {
            let mut c=Command::new("powershell.exe");
            c.args(["-NoProfile","-STA","-Command",UI])
                .env("MH_SETUP_TOOL",std::env::current_exe().map_err(|e|e.to_string())?)
                .env("MH_SETUP_ROOT",root).stdout(Stdio::inherit()).stderr(Stdio::inherit());
            if let Some(p)=a.preview {c.env("MH_SETUP_PREVIEW",p);}
            if let Some(p)=a.game {c.env("MORDHAU_DIR",p);}
            let result=c.status().map_err(|e|e.to_string())?;
            if !result.success() {return Err("Setup window could not start".into());}
            Ok(json!({"window_closed":true}))
        }
        _=>Err("Unknown operation".into()),
    }
}

fn main() {
    let result=args().and_then(run);
    match result {Ok(v)=>println!("{v}"),Err(e)=>{println!("{}",json!({"ready":false,"error":e}));std::process::exit(2);}}
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn missing_install_is_never_ready() {
        let v=check(Some(Path::new("definitely-no-installed-game")),Path::new("."));
        assert_eq!(v["installed"],false);assert_eq!(v["ready"],false);assert!(v["error"].is_string());
    }
    #[test] fn runtime_discovery_never_reaches_other_projects() {
        assert!(runtime(Path::new("definitely-no-framework-folder")).is_none());
    }
}

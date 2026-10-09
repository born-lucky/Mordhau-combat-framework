fn main() {
    let mut args: Vec<_> = std::env::args_os().skip(1).collect();
    let runtime = args.first().is_some_and(|a| a == "--runtime-preflight");
    if runtime { args.remove(0); }
    if args.len() > 1 {
        eprintln!("Usage: verify-install [--runtime-preflight] [MORDHAU-INSTALL-DIRECTORY]");
        std::process::exit(2);
    }
    let result = match args.first() {
        Some(p) => mh_install::Install::verify(std::path::Path::new(p)),
        None => mh_install::Install::discover(),
    };
    match result {
        Ok(i) => {
            println!("Verified supported original installation: {}", i.root().display());
            println!("Original EXE SHA1: {}", i.exe_sha1());
            if runtime {
                match mh_install::RuntimeInputs::local_from_environment(i) {
                    Ok(r) => println!("Local required input files checked: {}. No game was launched.", r.local_data.display()),
                    Err(e) => { eprintln!("Runtime preflight failed: {e}"); std::process::exit(2); }
                }
            } else { println!("Original files remain in place. Local import/cache readiness was NOT checked."); }
            println!("Private v1 release remains blocked: the complete local importer and licensing review are unfinished.");
        }
        Err(e) => { eprintln!("Install verification failed: {e}"); std::process::exit(2); }
    }
}

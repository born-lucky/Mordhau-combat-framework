//! print a Blueprint function's parameters (name, type, flags, sub type) as kismet.rs parses them
//!   cargo run -p mh-ui --example funcsig <package> <function>...
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let vfs = std::sync::Arc::new(mh_pak::Vfs::mount_default().expect("paks"));
    let rd = mh_pak::Reader::new(vfs);
    let pk = rd.open(&a[1]).expect("package");
    let (ci, _, members) = mh_ui::kismet::class_export(&rd, &pk).unwrap();
    let f = mh_ui::kismet::class_functions(&rd, &pk, ci);
    for n in &a[2..] {
        if n == "members" {
            for p in &members {
                println!("member {} {} {:?}", p.name, p.ty, p.sub.as_ref().map(|s| &s.name));
            }
            continue;
        }
        match f.get(n) {
            Some(func) => {
                for p in func.params() {
                    println!("{n}: {} {} flags={:#x} {:?}", p.name, p.ty, p.flags, p.sub.as_ref().map(|s| &s.name));
                }
            }
            None => println!("{n}: not found"),
        }
    }
}

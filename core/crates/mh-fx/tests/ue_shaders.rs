//! every generated particle shader (data_gen/shaders/particles, scripts/shaders/particle_wgsl.py) parses and validates
//! as WGSL inside the ue_material.rs prelude (Bevy's preprocessor directives resolved the way Bevy does for a
//! material without tonemapping in shader)

#[test]
fn generated_particle_shaders_validate() {
    let dir = mh_fx::ue_material::shader_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        eprintln!("no generated shaders in {} (run scripts/shaders/particle_wgsl.py)", dir.display());
        return;
    };
    // the Bevy-dependent block -> naga-only stand-ins (ue_material::UE_STUBS)
    let full = mh_fx::ue_material::UE_PRELUDE;
    let (b0, b1) = (full.find("//UE_BEVY_BEGIN").unwrap(), full.find("//UE_BEVY_END").unwrap());
    let prelude = format!("{}{}{}", &full[..b0], mh_fx::ue_material::UE_STUBS, &full[b1 + "//UE_BEVY_END".len()..])
        .replace("#import bevy_pbr::mesh_view_bindings::view", "struct V { clip_from_world: mat4x4<f32>, }\n@group(0) @binding(0) var<uniform> view: V;")
        .replace("#{MATERIAL_BIND_GROUP}", "2");
    // drop the TONEMAP_IN_SHADER blocks
    let mut pre = String::new();
    let mut skip = false;
    for l in prelude.lines() {
        let t = l.trim();
        if t.starts_with("#ifdef") {
            skip = true;
            continue;
        }
        if t.starts_with("#endif") {
            skip = false;
            continue;
        }
        if !skip {
            pre.push_str(l);
            pre.push('\n');
        }
    }
    let mut n = 0;
    let mut bad = vec![];
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("wgsl") {
            continue;
        }
        let body = mh_fx::ue_material::patch(&std::fs::read_to_string(&p).unwrap());
        // every scene-depth / ambient-volume sample is routed to the prelude's engine inputs
        for t in ["(scene_depth,", "(vol3d_1,", "(vol3d_2,"] {
            assert!(!body.contains(t), "{}: {t} left unpatched", p.display());
        }
        let src = format!("{pre}\n{body}");
        n += 1;
        match naga::front::wgsl::parse_str(&src) {
            Ok(m) => {
                let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
                if let Err(err) = v.validate(&m) {
                    bad.push(format!("{}: {err:?}", p.display()));
                }
            }
            Err(err) => bad.push(format!("{}: {}", p.display(), err.emit_to_string(&src))),
        }
    }
    assert!(bad.is_empty(), "{} of {n} shaders fail:\n{}", bad.len(), bad.join("\n").chars().take(6000).collect::<String>());
    eprintln!("{n} generated particle shaders validate");
}

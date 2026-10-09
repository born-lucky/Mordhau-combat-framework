//! mh-fx offscreen evidence: the combat effects read from the Blueprints (parry sparks = BP_MordhauWeapon
//! BlockParticles, clash = HitCancelParticles, flesh hit = P_BloodSplash, armour hit = BP_ArmorBloodSplash's system),
//! started through FxRequest in front of an offscreen Camera3d; two screenshots (just after the bursts, then later)
//! and summary.json under state/runtime_evidence/<date>/<time>-mh-fx/.
//!   sh scripts/cargo.sh run -p mh-fx --example evidence

use bevy::prelude::*;
use mh_fx::*;
use mh_ui::evidence::{offscreen_app, run_dir, screenshot};

#[derive(Resource)]
struct Run {
    frame: u32,
    dir: std::path::PathBuf,
    fx: Vec<(String, String)>,
    log: Vec<serde_json::Value>,
}

fn main() {
    let mut app = offscreen_app(true);
    let dir = run_dir("mh-fx");
    // `--seq` runs as the game does: no depth prepass (mh-runtime never inserts FxDepthPrepass)
    let prepass = !std::env::args().any(|a| a == "--seq");
    app.add_plugins(FxPlugin).insert_resource(FxGround(Some(0.0))).insert_resource(FxDepthPrepass(prepass)).insert_resource(Run { frame: 0, dir, fx: vec![], log: vec![] }).add_systems(Update, script);
    // a lit floor and a low wall the effects cross (depth fade / soft-particle evidence), lit by a UE-like sun
    // (intensity 5 lux, shadows) with UE exposure 1 (Bevy EV100 = -log2(1.2), as bevy-runtime lighting.rs maps it)
    let w = app.world_mut();
    let plane = w.resource_mut::<Assets<Mesh>>().add(Plane3d::default().mesh().size(4.0, 4.0));
    let wall = w.resource_mut::<Assets<Mesh>>().add(Cuboid::new(4.0, 1.2, 0.25));
    let mat = w.resource_mut::<Assets<StandardMaterial>>().add(StandardMaterial { base_color: Color::srgb(0.35, 0.35, 0.36), perceptual_roughness: 0.9, ..default() });
    w.spawn((Mesh3d(plane), MeshMaterial3d(mat.clone()), Transform::from_xyz(0.0, 0.0, 0.0)));
    w.spawn((Mesh3d(wall), MeshMaterial3d(mat), Transform::from_xyz(0.0, 0.6, -0.6)));
    w.spawn((DirectionalLight { illuminance: 5.0, shadow_maps_enabled: true, ..default() }, Transform::from_xyz(1.0, 3.0, 2.0).looking_at(Vec3::ZERO, Vec3::Y)));
    w.insert_resource(GlobalAmbientLight { color: Color::srgb(0.6, 0.7, 0.9), brightness: 0.4, affects_lightmapped_meshes: true });
    let cams: Vec<Entity> = w.query_filtered::<Entity, With<Camera3d>>().iter(w).collect();
    for c in cams {
        w.entity_mut(c).insert(bevy::camera::Exposure { ev100: -(1.2f32).log2() });
    }
    app.run();
}

fn script(world: &mut World) {
    let f = {
        let mut r = world.resource_mut::<Run>();
        r.frame += 1;
        r.frame
    };
    // `-- --trail`: a Longsword swing (sockets sweeping an arc) with its swing trail, a flesh hit's blood trail at
    // frame 75, and UAttackMotion::OnLeave's StopTrails(0.25) at frame 100 (shots at 92 and 118)
    if std::env::args().any(|a| a == "--trail") {
        trail_script(world, f);
        return;
    }
    if std::env::args().any(|a| a == "--seq") {
        seq_script(world, f);
        return;
    }
    match f {
        // frame 3: a warm-up round (materials, textures and pipelines get created); frame 90: the round that is shot
        3 | 90 => {
            let p = mh_ui::Paks::get_or_mount(world).expect("paks");
            let rd = mh_pak::Reader::new(p.0.clone());
            let c = CombatFx::read(&rd);
            let evs = [("parry", false), ("clash", false), ("hit", false), ("hit", true)];
            let xs = [-0.9f32, -0.3, 0.3, 0.9];
            let mut fx = vec![];
            // `-- --systems a,b,c`: those particle systems (map effects) instead of the combat events
            let sys_arg: Vec<String> = std::env::args().skip_while(|a| a != "--systems").nth(1).map(|s| s.split(',').map(str::to_string).collect()).unwrap_or_default();
            if !sys_arg.is_empty() {
                let n = sys_arg.len() as f32;
                for (i, sys) in sys_arg.iter().enumerate() {
                    let x = (i as f32 - (n - 1.0) / 2.0) * 0.9;
                    fx.push((sys.rsplit('/').next().unwrap_or("").to_string(), sys.clone()));
                    world.write_message(FxRequest { system: sys.clone(), pos: Vec3::new(x, 0.6, 0.0), dir: Vec3::Z });
                }
                world.resource_mut::<Run>().fx = fx;
                return;
            }
            for (i, (k, armoured)) in evs.iter().enumerate() {
                let ev = serde_json::json!({"kind": k});
                if let Some(sys) = c.for_event(&ev, *armoured) {
                    fx.push((format!("{k}{}", if *armoured { " (armour)" } else { "" }), sys.to_string()));
                    world.write_message(FxRequest { system: sys.to_string(), pos: Vec3::new(xs[i], 1.0, 0.0), dir: Vec3::Z });
                }
            }
            world.resource_mut::<Run>().fx = fx;
        }
        96 | 118 => {
            let name = if f == 96 { "burst.png" } else { "later.png" };
            let d = world.resource::<Run>().dir.join(name);
            screenshot(world, d);
            let s = world.resource::<FxStats>().clone();
            let v = serde_json::json!({"shot": name, "frame": f, "live_systems": s.live_systems, "live_particles": s.live_particles});
            world.resource_mut::<Run>().log.push(v);
        }
        500 => {
            let s = world.resource::<FxStats>().clone();
            let r = world.resource::<Run>();
            let dir = r.dir.clone();
            let ok = ["burst.png", "later.png"].iter().all(|p| dir.join(p).exists());
            let summary = serde_json::json!({
                "plugin": "mh-fx", "frames": f, "screenshots_written": ok,
                "events": r.fx.iter().map(|(k, s)| serde_json::json!({"event": k, "system": s})).collect::<Vec<_>>(),
                "records": r.log, "peak_particles": s.peak_particles, "live_systems_at_end": s.live_systems,
                "unsupported_modules": s.unsupported_modules, "missing": s.missing, "ground_y_m": 0.0,
            });
            let _ = std::fs::write(dir.join("summary.json"), serde_json::to_string_pretty(&summary).unwrap());
            println!("mh-fx evidence: {} (screenshots {ok}, peak {} particles, {} systems still live)", dir.display(), s.peak_particles, s.live_systems);
            world.write_message(AppExit::Success);
        }
        _ => {}
    }
}

fn trail_script(world: &mut World, f: u32) {
    let cfg = {
        let p = mh_ui::Paks::get_or_mount(world).expect("paks");
        let rd = mh_pak::Reader::new(p.0.clone());
        trail::TrailConfig::read(&rd, "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword")
    };
    if f == 60 {
        world.write_message(trail::TrailEvent::Start { weapon: 1, blood: false, duration: 0.5, config: cfg.clone() });
        world.resource_mut::<Run>().fx = vec![("swing".into(), cfg.swing_particles.clone()), ("blood".into(), cfg.blood_particles.clone())];
    }
    if f == 75 {
        world.write_message(trail::TrailEvent::Start { weapon: 1, blood: true, duration: cfg.blood_max_duration, config: cfg.clone() });
    }
    if f == 100 {
        world.write_message(trail::TrailEvent::Stop { weapon: 1, delay: 0.25 });
    }
    if (60..130).contains(&f) {
        // a horizontal right-to-left strike about a pivot 1.0 m up, blade from 0.25 m (TraceStart) to 1.1 m (TraceEnd)
        let ang = -1.2 + (f - 60) as f32 * 0.06;
        let dir = Vec3::new(ang.cos(), 0.15 * ang.sin(), -ang.sin()).normalize();
        let piv = Vec3::new(0.0, 1.0, 0.6);
        world.write_message(trail::TrailEvent::Sockets { weapon: 1, a: piv + dir * 0.25, b: piv + dir * 1.1 });
    }
    match f {
        92 | 118 => {
            let name = if f == 92 { "burst.png" } else { "later.png" };
            let d = world.resource::<Run>().dir.join(name);
            screenshot(world, d);
        }
        200 => {
            let r = world.resource::<Run>();
            let summary = serde_json::json!({"plugin": "mh-fx", "mode": "trail", "trail": format!("{cfg:?}"),
                "events": r.fx.iter().map(|(k, s)| serde_json::json!({"event": k, "system": s})).collect::<Vec<_>>()});
            let _ = std::fs::write(r.dir.join("summary.json"), serde_json::to_string_pretty(&summary).unwrap());
            println!("mh-fx evidence: {} (trail)", r.dir.display());
            world.write_message(AppExit::Success);
        }
        _ => {}
    }
}

/// `-- --seq [--old]`: a flesh hit, an armour hit, a parry and a clash, each spawned with the systems and rotation
/// the exe uses (CombatFx::for_event_all; hit: MakeFromXZ(-trace dir), block: ZeroRotator = +X), then a strip of
/// shots from 1 to 40 frames after the spawn. `--old` reproduces the pre-r12 mapping (clash -> HitCancelParticles,
/// one system). Shots: seq_<frame offset>.png
fn seq_script(world: &mut World, f: u32) {
    const SHOTS: [u32; 7] = [1, 3, 6, 10, 16, 25, 40];
    const SPAWN: u32 = 90;
    let old = std::env::args().any(|a| a == "--old");
    if f == 3 || f == SPAWN {
        let p = mh_ui::Paks::get_or_mount(world).expect("paks");
        let rd = mh_pak::Reader::new(p.0.clone());
        let c = CombatFx::read(&rd);
        // a right-to-left swing toward the camera: the blood faces back along it
        let trace = Vec3::new(-1.0, -0.2, 0.4).normalize();
        let evs = [("hit", false), ("hit", true), ("parry", false), ("clash", false)];
        let xs = [-1.05f32, -0.35, 0.35, 1.05];
        let mut fx = vec![];
        for (i, (k, armoured)) in evs.iter().enumerate() {
            let ev = serde_json::json!({"kind": k});
            let systems: Vec<String> = if old && *k == "clash" { vec![c.hit_cancel.clone()] } else { c.for_event_all(&ev, *armoured).into_iter().map(String::from).collect() };
            let dir = if *k == "hit" { CombatFx::hit_dir(trace) } else { Vec3::X };
            for sys in systems {
                fx.push((format!("{k}{}", if *armoured { " (armour)" } else { "" }), sys.clone()));
                world.write_message(FxRequest { system: sys, pos: Vec3::new(xs[i], 1.0, 0.0), dir });
            }
        }
        if f == SPAWN {
            world.resource_mut::<Run>().fx = fx;
        }
    }
    if f > SPAWN && SHOTS.contains(&(f - SPAWN)) {
        let name = format!("seq_{:02}.png", f - SPAWN);
        let d = world.resource::<Run>().dir.join(&name);
        screenshot(world, d);
        let s = world.resource::<FxStats>().clone();
        let t = world.resource::<Time>().elapsed_secs();
        world.resource_mut::<Run>().log.push(serde_json::json!({"shot": name, "frame": f, "t": t, "live_systems": s.live_systems, "live_particles": s.live_particles}));
    }
    if f == SPAWN + 120 {
        let s = world.resource::<FxStats>().clone();
        let r = world.resource::<Run>();
        let summary = serde_json::json!({"plugin": "mh-fx", "mode": if old { "seq-old" } else { "seq" },
            "ue_shaders": std::env::var("MH_FX_UE_SHADERS").unwrap_or_default(),
            "events": r.fx.iter().map(|(k, s)| serde_json::json!({"event": k, "system": s})).collect::<Vec<_>>(),
            "records": r.log, "peak_particles": s.peak_particles, "unsupported_modules": s.unsupported_modules, "missing": s.missing});
        let _ = std::fs::write(r.dir.join("summary.json"), serde_json::to_string_pretty(&summary).unwrap());
        println!("mh-fx evidence: {} (seq)", r.dir.display());
        world.write_message(AppExit::Success);
    }
}

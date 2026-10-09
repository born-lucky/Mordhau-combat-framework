//! Simulated proxies on a real map: mh_level::collision::CollisionWorld (the game's placed collision, landscape and
//! blocking volumes, read from the user's install) through mh-character's `World`. A pawn sprints across Camp's
//! landscape (slopes, bumps) in several directions; the proxy of it gets ReplicatedMovement (quantized as a pawn's
//! FRepMovement) at 10 Hz and runs SimulateMovement on the same collision in between (movement.rs ProxyMove). Its
//! vertical error stays at the floor's change between updates, where the r4 flat-ground extrapolation (velocity on the
//! last floor height) floats above or sinks into the slope. SKIP (pass) without the install or the records.

use std::path::Path;
use std::sync::Arc;

use mh_character::exe_cmc::AVG_FLOOR_DIST;
use mh_character::uemath::{size_sq, sub, v};
use mh_character::{CharacterRecords, CharacterSource, ExeInput, ExeMovement, RecordsJson, World};
use mh_level::collision::CollisionWorld;
use mh_level::{read, Pkgs};
use mh_net::movement::{rep_movement_of, ProxyMove, SmoothCfg};
use mh_pak::{Reader, Vfs};

const DT: f32 = 1.0 / 60.0;

fn records() -> Option<CharacterRecords> {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../core/tests/golden/character/records.json");
    Some(RecordsJson(&std::fs::read_to_string(p).ok()?).load().expect("records.json"))
}

fn camp() -> Option<(mh_level::LevelData, CollisionWorld)> {
    let vfs = match Vfs::mount_default() {
        Ok(v) => Arc::new(v),
        Err(e) => {
            eprintln!("SKIP: {e}");
            return None;
        }
    };
    let pkg = vfs.list().find(|p| p.to_lowercase().ends_with("/camp.umap"))?.to_string();
    let pk = Pkgs::new(Reader::new(vfs.clone()));
    let d = read(&pk, pkg.trim_end_matches(".umap"));
    let w = CollisionWorld::build(&pk, &d, None);
    Some((d, w))
}

#[test]
fn proxy_walks_camp_landscape() {
    let Some(r) = records() else { return eprintln!("SKIP: records.json missing") };
    let Some((d, w)) = camp() else { return };
    let Some(s) = d.spawns.first() else { return };
    let p = s.xf.translation();
    let c = SmoothCfg::default();
    let (mut zerr, mut flat_zerr, mut err) = (vec![], vec![], vec![]);
    for yaw in [0.0f32, 90.0, 180.0, 270.0, 45.0, 225.0] {
        let top = v(p[0] as f32, p[1] as f32, p[2] as f32 + 50.0);
        let Some(h) = w.line_trace(top, v(p[0] as f32, p[1] as f32, p[2] as f32 - 2000.0)) else { continue };
        let mut m = ExeMovement::new(&r, v(p[0] as f32, p[1] as f32, h.impact_point.z + 96.0 + AVG_FLOOR_DIST));
        m.world_time = 10.0;
        m.frame(&w, DT, &ExeInput::default());
        let mut proxy = ProxyMove::new(&rep_movement_of(&m), ExeMovement::new(&r, m.location));
        let (mut fl, mut fv) = (m.location, m.velocity);
        for k in 0..60 * 6 {
            m.frame(&w, DT, &ExeInput { fwd: 1.0, sprint: true, yaw, ..Default::default() });
            if k % 6 == 0 {
                let rep = rep_movement_of(&m);
                proxy.on_rep(&rep, &c);
                fl = v(rep.location[0], rep.location[1], rep.location[2]);
                fv = v(rep.velocity[0], rep.velocity[1], 0.0);
            }
            fl = v(fl.x + fv.x * DT, fl.y + fv.y * DT, fl.z);
            proxy.tick(&w, DT, &c);
            if m.mode == mh_character::Mode::Walking && m.location.z as f64 > w.kill_z {
                zerr.push((proxy.location().z - m.location.z).abs());
                flat_zerr.push((fl.z - m.location.z).abs());
                err.push(size_sq(sub(proxy.location(), m.location)).sqrt());
            }
        }
    }
    assert!(err.len() > 600, "too few walking samples ({})", err.len());
    let pct = |xs: &mut Vec<f32>, q: f32| {
        xs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        xs[((xs.len() - 1) as f32 * q) as usize]
    };
    let (z95, f95, e95) = (pct(&mut zerr, 0.95), pct(&mut flat_zerr, 0.95), pct(&mut err, 0.95));
    let (zmax, fmax) = (pct(&mut zerr, 1.0), pct(&mut flat_zerr, 1.0));
    eprintln!("Camp proxy: z error p95 {z95:.2} max {zmax:.2} cm (flat model p95 {f95:.2} max {fmax:.2}); 3D p95 {e95:.2} cm over {} samples", err.len());
    assert!(z95 <= f95 && zmax < fmax, "proxy floor handling no better than flat extrapolation");
    assert!(z95 < 5.0, "proxy z error p95 {z95}");
}

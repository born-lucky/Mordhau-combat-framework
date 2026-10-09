//! Smoke: every mode map (TDM / SKM / FFA / FL / INV / HRD / SG / DIH / DU / TF / BR / SC / Tourney / Moshpit /
//! Truce prefixes) builds its World, begins play and ticks 60 s at 60 Hz without a panic. A probe character stands
//! in every actor's overlaps for one second (frames 600..660) so every trigger, pushable, door leaf and ladder
//! sees a begin and an end overlap; the use key and a kick hit every interactable / damageable actor once.
//! SKIP (pass) without the install.

use mh_level::{read, Pkgs};
use mh_pak::{Reader, Vfs};
use mh_world::{ActorId, CharId, CharView, DamageInfo, Queries, SpawnedStatus, World};
use std::cell::Cell;
use std::sync::Arc;

struct Probe {
    on: Cell<bool>,
}
impl Queries for Probe {
    fn overlapping_chars(&self, _: ActorId) -> Vec<CharId> {
        if self.on.get() { vec![1] } else { vec![] }
    }
    fn overlapping_actors(&self, _: ActorId) -> Vec<ActorId> {
        vec![]
    }
    fn spawned_status(&self, _: ActorId) -> SpawnedStatus {
        SpawnedStatus { exists: true, ..Default::default() }
    }
}

const PREFIXES: [&str; 15] = ["TDM_", "SKM_", "FFA_", "FL_", "INV_", "HRD_", "SG_", "DIH_", "DU_", "TF_", "BR_", "SC_", "Tourney", "Moshpit", "Truce"];

#[test]
fn every_mode_map_ticks_60s() {
    let Some(v) = Vfs::mount_default().ok() else { return eprintln!("SKIP: no install") };
    let maps: Vec<String> = v
        .list()
        .filter(|p| p.contains("/Maps/") && p.ends_with(".umap"))
        .map(|p| p.trim_end_matches(".umap").to_string())
        .filter(|p| {
            let n = p.rsplit('/').next().unwrap_or("");
            PREFIXES.iter().any(|x| n.starts_with(x))
        })
        .collect();
    // a debug build runs every 8th map (the full sweep is `cargo test -p mh-world --release --test smoke`, 157 maps
    // in about 4 minutes)
    let maps: Vec<String> = if cfg!(debug_assertions) { maps.into_iter().step_by(8).collect() } else { maps };
    let pk = Pkgs::new(Reader::new(Arc::new(v)));
    let probe = Probe { on: Cell::new(false) };
    let (mut total_actors, mut total_events) = (0usize, 0usize);
    for m in &maps {
        let d = read(&pk, m);
        let mut w = World::from_level(&pk, &d);
        w.fl_match_in_progress = true;
        w.game_mode_respawn = Some((30.0, 30.0, 30.0));
        let mut n = w.begin_play(&probe).len();
        let ch = CharView { id: 1, team: Some(0), allow_vehicles: true, has_player_controller: true, ..Default::default() };
        for f in 0..3600 {
            probe.on.set((600..660).contains(&f));
            if f == 900 {
                for a in 0..w.actors.len() {
                    n += w.interact(a, &ch).len();
                    n += w.held_interact(a, &ch).len();
                    let kick = DamageInfo { causer: Some(1), instigator: Some(1), instigator_player: true, instigator_team: Some(1), attack_move: Some(4), causer_forward: Some([1.0, 0.0, 0.0]), ..Default::default() };
                    n += w.apply_damage(a, 30.0, &kick, &probe, std::slice::from_ref(&ch)).len();
                    n += w.component_hit(a, &ch).len();
                }
            }
            n += w.tick(1.0 / 60.0, &probe, std::slice::from_ref(&ch)).len();
        }
        total_actors += w.actors.len();
        total_events += n;
        println!("{}: {} actors, {} triggers, {n} events", m.rsplit('/').next().unwrap(), w.actors.len(), w.triggers.len());
        pk.clear();
    }
    println!("{} maps, {total_actors} actors, {total_events} events", maps.len());
    assert!(maps.len() >= if cfg!(debug_assertions) { 12 } else { 100 }, "{}", maps.len());
}

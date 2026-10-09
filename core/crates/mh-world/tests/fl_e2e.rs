//! Frontline end to end on FL_Camp: mh-world's BP_CapturePoint objective layer driving mh-mode's
//! BP_FrontlineGameMode through `fl_bridge::FlBridge`. The mode comes from the spec matrix (ModeData::from_spec "FL",
//! the FL_Camp control points) and the world from the paks. SKIP (pass) without either.

use mh_level::{read, Pkgs};
use mh_mode::{GameMode, ModeData};
use mh_pak::{Reader, Vfs};
use mh_world::fl_bridge::FlBridge;
use mh_world::frontline::ObjectiveKind;
use mh_world::{ActorId, CharId, CharView, Kind, Queries, SpawnedStatus, World, WorldEvent};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

#[derive(Default)]
struct Q {
    chars: BTreeMap<ActorId, Vec<CharId>>,
}
impl Queries for Q {
    fn overlapping_chars(&self, a: ActorId) -> Vec<CharId> {
        self.chars.get(&a).cloned().unwrap_or_default()
    }
    fn overlapping_actors(&self, _: ActorId) -> Vec<ActorId> {
        vec![]
    }
    fn spawned_status(&self, _: ActorId) -> SpawnedStatus {
        SpawnedStatus::default()
    }
}

/// FL_Camp: the world's capture points match mh-mode's by name. With the prerequisites the mode reports, a pushed
/// wagon raises its point's ObjectiveProgress; the bridge turns the BP_CapturePoint's SetCaptureProgress into the
/// mode's control point capture progress (hidden point: the enemy's progress = ObjectiveProgress)
#[test]
fn fl_camp_wagon_drives_the_mode() {
    let d = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../data_gen/spec");
    if !d.join("index.json").exists() {
        return eprintln!("SKIP: no spec");
    }
    let Some(v) = Vfs::mount_default().ok() else { return };
    let pk = Pkgs::new(Reader::new(Arc::new(v)));
    let spec = mh_spec::Spec::load(&d, false).expect("spec");
    let data = ModeData::from_spec(&spec, "FL").expect("FL");
    let k = Arc::new(mh_mode::spec_bots::kismet(&spec).expect("kismet"));
    let mut gm = GameMode::new(data, k, mordhau_core::ue::CrtRand::new(1));
    let names: Vec<String> = gm.control_points.iter().map(|c| c.name.clone()).collect();
    let level = read(&pk, "Mordhau/Content/Mordhau/Maps/DuelCamp/FL_Camp");
    let mut w = World::from_level(&pk, &level);
    let mut br = FlBridge::new(&w, &gm);
    println!("mode points {names:?}; world points {:?}; matched {}", w.cps.keys().map(|c| &w.actors[*c].name).collect::<Vec<_>>(), br.cp_index.len());
    assert!(!br.cp_index.is_empty());
    let q0 = Q::default();
    let ev = w.begin_play(&q0);
    br.sync(&gm, &mut w);
    br.apply(&mut gm, &ev, 0.0);
    // the wagon and its point
    let wagon = w.actors.iter().find(|a| w.is_pushable_objective(a.id) && matches!(&a.kind, Kind::Objective(o) if o.capture_point.is_some_and(|c| br.cp_index.contains_key(&c)))).expect("wagon").id;
    let cp = match &w.actors[wagon].kind {
        Kind::Objective(o) => o.capture_point.unwrap(),
        _ => unreachable!(),
    };
    let mi = br.cp_index[&cp];
    let (team, hidden) = {
        let Kind::Objective(o) = &w.actors[wagon].kind else { unreachable!() };
        assert_eq!(o.kind, ObjectiveKind::Pushable);
        (if o.push.as_ref().unwrap().team1_curve.is_some() { 0u8 } else { 1u8 }, w.cps[&cp].hidden)
    };
    println!("wagon {} -> point {} (mode #{mi}, hidden {hidden}, owner {:?}); pusher team {team}", w.actors[wagon].name, w.actors[cp].name, w.cps[&cp].owning_team);
    // the enemy has the prerequisites (as the mode's tick would raise them)
    w.capture_point_prerequisites(cp, true);
    let mut q = Q::default();
    q.chars.insert(wagon, vec![1]);
    let pusher = CharView { id: 1, team: Some(team), ..Default::default() };
    let p0 = gm.control_points[mi].capture_progress;
    let mut seen = vec![];
    for _ in 0..(60 * 5) {
        gm.tick(1.0 / 60.0);
        br.sync(&gm, &mut w);
        let ev = w.tick(1.0 / 60.0, &q, &[pusher.clone()]);
        seen.extend(ev.iter().filter(|e| matches!(e, WorldEvent::CaptureProgress { .. })).cloned());
        br.apply(&mut gm, &ev, 1.0 / 60.0);
    }
    let op = w.cps[&cp].objective_progress;
    let p1 = gm.control_points[mi].capture_progress;
    println!("ObjectiveProgress {op:.5}; mode capture progress {p0:.5} -> {p1:.5}; {} CaptureProgress events", seen.len());
    assert!(op > 0.0 && !seen.is_empty());
    let Some(WorldEvent::CaptureProgress { progress, .. }) = seen.last() else { unreachable!() };
    // SetCaptureProgress stores the clamped value; ReplicatedCaptureProgress quantizes it (the mode's own tick may
    // move it further when pawns stand in its area: none here)
    assert!((p1 - progress).abs() < 1e-6, "{p1} vs {progress}");
}

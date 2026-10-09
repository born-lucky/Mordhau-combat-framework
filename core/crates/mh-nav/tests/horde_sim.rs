//! Horde end to end (rust-mode-ai r5): HRD_Camp (collision + the cooked navmesh), mh-mode's Horde GameMode
//! (ModeData::from_spec("HRD")) inside mh-sim, two player controllers whose pawns are played by BT_Deathmatch Knight
//! bots, and the waves' enemies spawned from the mode's horde_spawn events at the map's BP_HordeSpawn actors as
//! BP_HordeAIController bots (BT_Horde read from the paks by mh_mode::ai::bt_read, their BOTBEHAVIOR_* profile, their
//! drawn equipment). Enemy deaths go back to the mode (GameMode::horde_enemy_killed with the last attacker's
//! controller); chests / graves / shop stay mode events for the host (counted here). Plays waves 1-3. SKIP without the
//! install / spec / character records.
//!
//! Test scaffolding (not game rules, UNCONFIRMED): the spawner order = the level's actor order (GetAllActorsOfClass);
//! the squad offset is added unrotated; ranged / thrown equipment (not simulated by the combat World) is replaced by
//! the enemy's first melee item, else BP_FistsWeapon; an enemy pawn starts on the floor under its spawner.

use mh_character::{CharacterSource, RecordsJson};
use mh_level::collision::CollisionWorld;
use mh_level::{read, Pkgs};
use mh_mode::ai::{BotBody, Bots, InventoryItem};
use mh_mode::event::Arg;
use mh_mode::game_mode::GameMode;
use mh_nav::Bounds;
use mh_pak::{Reader, Vfs};
use mh_sim::{FighterDesc, Sim};
use mordhau_core::ue::{CrtRand, FVector};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

const LS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn wpn_path(m: &mh_spec::Spec, name: &str) -> Option<String> {
    let e = m.entities.get(&format!("ENT_WPN_{name}"))?;
    m.sources.get(&e.source).map(|s| s.path.clone())
}

fn melee_ok(p: &str) -> bool {
    let n = p.rsplit('/').next().unwrap_or(p);
    !["Bow", "Crossbow", "Throw", "Rock", "Turd", "ThrowingKnife", "Javelin"].iter().any(|k| n.contains(k))
}

fn body_for(s: &Sim, fi: usize, walk: f64) -> BotBody {
    let mut body = BotBody::new(&s.combat.fighters[fi].name);
    body.location = s.movers[fi].location;
    body.yaw = s.yaw[fi] as f64;
    body.team = s.combat.fighters[fi].team;
    body.weapon_length = s.combat.fighters[fi].tracer.length;
    body.max_walk_speed = walk;
    body.pawn = mh_mode::ai::combat_host::pawn_view(&s.combat, fi);
    body.weapon = mh_mode::ai::combat_host::weapon_view(&s.combat, fi);
    body.inventory = vec![InventoryItem { weapon: body.weapon.clone(), is_fists: false, ranged: false }];
    body
}

/// waves 1-3 (ignored until rust-character fixes the landscape stall, `repro_landscape_stall` below: on HRD_Camp some
/// enemies freeze on walkable landscape on their way from the spawners, so wave 2 never clears). r6 diagnosis of the r5
/// 0-kill run: the weapons never swept (no pose source in the host loop; the clip poser now poses the attacks), and
/// enemies spawned into rocks (moved onto the navmesh). MH_HORDE_DEBUG=1 dumps the bots every 10 s,
/// MH_HORDE_WATCH=<name> one bot's movement every second.
#[test]
#[ignore]
fn horde_waves_with_bots_on_camp() {
    horde_run(3, 6 * 60);
}

/// wave 1 cleared by the players' bots (the enemies spawned, fought and were killed over the cooked navmesh), with
/// the game's perception: sight (2600 cm, 65 deg, line of sight) and hearing (character sounds -> noise events,
/// mh-sim noise.rs; r6: without hearing two enemies idled unperceived in a hollow below the players)
#[test]
fn horde_wave_one_with_bots_on_camp() {
    horde_run(1, 3 * 60);
}

fn horde_run(target_wave: i64, max_secs: usize) {
    let Ok(vfs) = Vfs::mount_default().map(Arc::new) else { return eprintln!("SKIP: no paks") };
    let Ok(matrix) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return eprintln!("SKIP: no spec") };
    let Ok(rec) = std::fs::read_to_string(repo().join("core/tests/golden/character/records.json")) else { return eprintln!("SKIP: no character records") };
    let records = RecordsJson(&rec).load().unwrap();
    let pkg = vfs.list().find(|p| p.to_lowercase().ends_with("/hrd_camp.umap")).unwrap().trim_end_matches(".umap").to_string();
    let pk = Pkgs::new(Reader::new(vfs.clone()));
    let d = read(&pk, &pkg);
    let src = mh_assets::pak_source::PakSource::new(vfs.clone());
    let tri = |p: &str| -> Option<Vec<[f64; 3]>> {
        let m = mh_assets::static_mesh::lod0(&src, p).ok()?;
        Some(m.indices.iter().map(|&i| m.vertices.positions[i as usize].map(|c| c as f64)).collect())
    };
    let w = CollisionWorld::build(&pk, &d, Some(&tri));
    let bounds: Vec<Bounds> = d.volumes.iter().filter(|v| v.class.contains("NavMeshBoundsVolume")).map(|v| Bounds { min: v.min.map(|c| c as f32), max: v.max.map(|c| c as f32) }).collect();
    let cooked = mh_nav::cooked::find(&pk, &pkg).is_some();
    let nav = mh_nav::cooked::nav_for_map(&pk, &pkg, &w, &bounds);
    // the map's BP_HordeSpawn actors (host spawner ids, actor order), dropped to the floor
    let spawners: Vec<FVector> = d
        .actors
        .iter()
        .filter(|a| a.class == "BP_HordeSpawn_C")
        .filter_map(|a| a.xf.as_ref().map(|x| x.translation()))
        .map(|t| FVector::new(t[0] as f32, t[1] as f32, t[2] as f32))
        .collect();
    assert!(!spawners.is_empty(), "HRD_Camp has BP_HordeSpawn actors");
    let floor = |w: &dyn mh_character::world::World, p: FVector| -> FVector {
        match w.line_trace(FVector::new(p.x, p.y, p.z + 100.0), FVector::new(p.x, p.y, p.z - 1000.0)) {
            Some(h) => FVector::new(p.x, p.y, h.impact_point.z + 100.0),
            None => p,
        }
    };

    // the enemies' weapons (melee only) + the players' longsword
    let hd = mh_mode::spec_horde::horde_data(&matrix).unwrap();
    let fists = wpn_path(&matrix, "BP_FistsWeapon").expect("BP_FistsWeapon in the spec");
    let loadable = |p: &str| !p.is_empty() && melee_ok(p) && matrix.entities.contains_key(&format!("ENT_WPN_{}", p.rsplit('/').next().unwrap()));
    let mut weapons: Vec<String> = vec![LS.to_string(), fists.clone()];
    for e in &hd.enemies {
        for p in e.equipment.iter().chain(&e.secondary_equipment) {
            if loadable(p) && !weapons.contains(p) {
                weapons.push(p.clone());
            }
        }
    }
    let bodies = w.bodies.clone();
    let wrefs: Vec<&str> = weapons.iter().map(|s| s.as_str()).collect();
    let ld = mh_sim::load::load(&matrix, vfs.clone(), &wrefs).expect("load");
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), records, Box::new(w), 1.0 / 60.0);
    s.nav = Some(nav);
    s.noise_loudness = mh_sim::noise::NoiseLoudness::read_with_weapons(&Reader::new(vfs.clone()), &wrefs);
    eprintln!("noise loudness {:?}; {} equipment sounds", s.noise_loudness.by_kind, s.noise_loudness.by_weapon.len());

    let k = Arc::new(mh_mode::spec_bots::kismet(&matrix).unwrap());
    let data = mh_mode::data::ModeData::from_spec(&matrix, "HRD").unwrap();
    let mut gm = GameMode::new(data, k.clone(), CrtRand::new(1));
    gm.set_horde_spawners(spawners.len());
    s.set_mode(gm);
    // the combat damage rules of BP_HordeGameMode from the spec (ENT_MODE_HRD rules_*: TeamDamageFactor 0.0 from
    // the CDO, extract/json .../GameModes/Horde/BP_HordeGameMode.json): no team damage between the horde's enemies
    s.combat.set_game_mode("Mordhau/Content/Mordhau/Blueprints/GameModes/Horde/BP_HordeGameMode", "Mordhau/Content/Mordhau/Blueprints/GameModes/Horde/BP_HordeGameState");
    assert_eq!(s.combat.mode_rules.as_ref().unwrap().team_damage_factor, 0.0);
    let players = ["P0", "P1"];
    for p in players {
        s.join(p, false, LS, "").unwrap();
    }
    s.bots = Some(Bots { k: k.clone(), team_mode: true, ..Default::default() });

    // trees and profiles
    let rd = Reader::new(vfs.clone());
    let loader = |p: &str| rd.read(p);
    let bt_horde = mh_mode::ai::bt_read::tree_def(&loader, "Mordhau/Content/Mordhau/Blueprints/GameModes/Horde/AI/BT_Horde").expect("BT_Horde");
    let bt_dm = mh_mode::spec_bots::tree(&matrix, "BT_Deathmatch").unwrap();
    let knight = mh_mode::spec_bots::profile(&matrix, "BOTBEHAVIOR_Knight").unwrap();

    let mut enemy_of: HashMap<String, usize> = HashMap::new();
    let mut last_hit: HashMap<String, String> = HashMap::new();
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    let mut spawned = 0usize;
    let mut kills_by_players = 0usize;
    let mut max_wave = -1;
    let mut subs = 0usize;
    let mut moved_spawns = 0usize;
    let proj = mh_nav::cooked::load(&pk, &pkg).and_then(|r| r.ok());
    let mut hit_pairs: BTreeMap<(String, String), usize> = BTreeMap::new();
    let mut kinds: BTreeMap<String, usize> = BTreeMap::new();
    let mut motion_samples: Vec<String> = vec![];
    let mut attack_geo: Vec<String> = vec![];
    let mut motions: BTreeMap<(String, String), usize> = BTreeMap::new();
    let walk = 300.0;
    let dt = 1.0 / 60.0;
    // the attack clips pose the fighters (the host's animation; without it the weapons never sweep: the r5 0-kill run)
    let mut poser = mh_sim::anim::ClipPoser::new(mh_assets::pak_source::PakSource::new(vfs.clone()));
    for frame in 0..(60 * max_secs) {
        let inp = s.bot_inputs();
        if let Ok(wn) = std::env::var("MH_HORDE_WATCH") {
            if frame % 60 == 0 {
                if let (Some(fi), Some(b)) = (s.combat.fighter_index(&wn), s.bots.as_ref()) {
                    let bi = b.bodies.iter().position(|x| x.name == wn).unwrap();
                    let i = inp.iter().find(|x| x.0 == fi).map(|x| (x.1.fwd, x.1.right, x.1.yaw));
                    let m = &s.movers[fi];
                    if let Some(Some(t)) = s.bot_steer.get(bi) {
                        for dz in [-60.0f32, 0.0, 60.0] {
                            let a = FVector::new(m.location.x, m.location.y, m.location.z + dz);
                            let d = FVector::new(t.x - a.x, t.y - a.y, 0.0);
                            let l = (d.x * d.x + d.y * d.y).sqrt().max(1.0);
                            let b = FVector::new(a.x + d.x / l * 150.0, a.y + d.y / l * 150.0, a.z);
                            if let Some(h) = s.collision.line_trace(a, b) {
                                let body = h.component.and_then(|c| bodies.get(c as usize));
                                eprintln!("   blocked dz {dz} at {:.0} cm by {:?}", h.distance, body.map(|b| (&b.name, &b.mesh, &b.profile, &b.object_type, b.kind)));
                            }
                        }
                    }
                    eprintln!("   mode {:?} motion {:?} floor body {:?} accel {:?}", m.mode, s.combat.fighters[fi].motion.map(|id| s.combat.m(fi, id).kind().to_string()), m.current_floor.hit.component.and_then(|c| bodies.get(c as usize)).map(|b| (&b.name, &b.mesh, &b.source)), m.acceleration);
                    eprintln!("watch {wn} t={:.0} at {:?} vel {:?} steer {:?} inp {:?} path {:?}", s.combat.now, m.location, m.velocity, s.bot_steer.get(bi), i, s.bot_follow.get(bi).map(|f| (f.idx, f.path.len(), f.path.first().copied())));
                }
            }
        }
        poser.pose(&mut s);
        s.step(&inp);
        // combat events: who hit whom, deaths
        let (hits, _) = s.drain();
        for e in &hits {
            let g = |k: &str| e.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
            *kinds.entry(g("kind")).or_default() += 1;
            if g("kind") == "motion" && motion_samples.len() < 12 {
                motion_samples.push(serde_json::to_string(e).unwrap());
            }
            if g("kind") == "motion" && g("motion") == "Attack" && g("who").starts_with('P') && attack_geo.len() < 40 {
                if let Some(fi) = s.combat.fighter_index(&g("who")) {
                    let me = s.movers[fi].location;
                    let best = (0..s.combat.fighters.len())
                        .filter(|&o| s.combat.fighters[o].team != s.combat.fighters[fi].team && !s.combat.fighters[o].dead)
                        .map(|o| (o, s.movers[o].location))
                        .min_by(|a, b| (a.1 - me).length().partial_cmp(&(b.1 - me).length()).unwrap());
                    if let Some((o, l)) = best {
                        let d = l - me;
                        let ang = d.y.atan2(d.x).to_degrees() - s.yaw[fi];
                        attack_geo.push(format!("{} t={:.1} -> {} d2d {:.0} dz {:.0} yaw-off {:.0} len {:.0}", g("who"), s.combat.now, s.combat.fighters[o].name, (d.x * d.x + d.y * d.y).sqrt(), d.z, ((ang + 540.0) % 360.0) - 180.0, s.combat.fighters[fi].tracer.length));
                    }
                }
            }
            if g("kind") == "motion" {
                let who = e.get("who").or_else(|| e.get("fighter")).and_then(|v| v.as_str()).unwrap_or("?").to_string();
                let what = e.get("motion").or_else(|| e.get("class")).or_else(|| e.get("name")).and_then(|v| v.as_str()).unwrap_or("?").to_string();
                *motions.entry((who, what)).or_default() += 1;
            }
            match e.get("kind").and_then(|v| v.as_str()) {
                Some("hit") => {
                    *hit_pairs.entry((g("attacker"), g("victim"))).or_default() += 1;
                    last_hit.insert(g("victim"), g("attacker"));
                }
                Some("died") => {
                    let who = g("who");
                    if let Some(&id) = enemy_of.get(&who) {
                        let killer = last_hit.get(&who).cloned();
                        let m = s.mode.as_mut().unwrap();
                        let kc = killer.as_deref().and_then(|k| m.gm.by_name(k));
                        if kc.is_some() {
                            kills_by_players += 1;
                        }
                        m.gm.horde_enemy_killed(id, kc);
                    }
                }
                _ => {}
            }
        }
        // mode events: player pawns, horde spawns, extras for the host
        for e in s.drain_mode() {
            *counts.entry(e.kind).or_default() += 1;
            match e.kind {
                "spawn_pawn" => {
                    let who = e.get("who").map(|a| a.as_s().to_string()).unwrap_or_default();
                    let bots = s.bots.as_ref().unwrap();
                    if players.contains(&who.as_str()) && !bots.bodies.iter().any(|b| b.name == who) {
                        let fi = s.combat.fighter_index(&who).unwrap();
                        let body = body_for(&s, fi, walk);
                        let mut bots = s.bots.take().unwrap();
                        let bi = bots.add_body(body);
                        s.bot_fighter.push(fi);
                        let now = s.combat.now;
                        let mut host = mh_mode::ai::combat_host::CombatHost { world: &mut s.combat, fighter_of: s.bot_fighter.clone() };
                        bots.add_bot(bi, knight.clone(), Some(&bt_dm), now, &mut host);
                        s.bots = Some(bots);
                    }
                }
                "horde_spawn" => {
                    let gi = |k: &str| e.get(k).map(|a| a.as_i()).unwrap_or(0);
                    let (id, sp) = (gi("enemy") as usize, gi("spawner") as usize);
                    let key = e.get("key").map(|a| a.as_s().to_string()).unwrap_or_default();
                    let off = match e.get("offset") {
                        Some(Arg::V(v)) => *v,
                        _ => [0.0; 3],
                    };
                    let def = hd.enemies.iter().find(|x| x.key == key).unwrap();
                    let eq = gi("equipment");
                    let drawn = if eq >= 0 { def.equipment.get(eq as usize).cloned().unwrap_or_default() } else { String::new() };
                    let weapon = if loadable(&drawn) {
                        drawn
                    } else {
                        subs += 1;
                        def.equipment.iter().find(|p| loadable(p)).cloned().unwrap_or_else(|| fists.clone())
                    };
                    let at = spawners[sp.min(spawners.len() - 1)];
                    let mut loc = floor(&*s.collision, FVector::new(at.x + off[0], at.y + off[1], at.z));
                    // a spot inside the collision (the offset grid reaches into rocks / walls): the nearest navmesh
                    // point instead (UE's spawn collision handling: UNCONFIRMED)
                    if s.collision.overlap_capsule(loc, 42.0, 96.0) {
                        if let Some((_, q)) = proj.as_ref().and_then(|m| m.project(loc)) {
                            loc = FVector::new(q.x, q.y, q.z + 100.0);
                            moved_spawns += 1;
                        }
                    }
                    let name = format!("E{id}");
                    let fi = s.add_fighter(&FighterDesc { name: name.clone(), weapon, left: String::new(), team: 1, location: loc, yaw: 0.0 });
                    enemy_of.insert(name.clone(), id);
                    spawned += 1;
                    let body = body_for(&s, fi, walk);
                    let behavior = def.behavior.rsplit('/').next().unwrap_or("BOTBEHAVIOR_Peasant").to_string();
                    let prof = mh_mode::spec_bots::profile(&matrix, &behavior).unwrap_or_else(|_| knight.clone());
                    let mut bots = s.bots.take().unwrap();
                    let bi = bots.add_body(body);
                    s.bot_fighter.push(fi);
                    let now = s.combat.now;
                    let mut host = mh_mode::ai::combat_host::CombatHost { world: &mut s.combat, fighter_of: s.bot_fighter.clone() };
                    bots.add_bot(bi, prof, Some(&bt_horde), now, &mut host);
                    s.bots = Some(bots);
                }
                _ => {}
            }
        }
        // fighter indices shift when a pawn is replaced: the bodies follow their fighters by name
        if let Some(b) = &s.bots {
            let idx: Vec<usize> = b.bodies.iter().map(|x| s.combat.fighter_index(&x.name).unwrap_or(0)).collect();
            s.bot_fighter = idx;
        }
        if std::env::var("MH_HORDE_DEBUG").is_ok() && frame % 600 == 0 {
            if let Some(b) = &s.bots {
                for (bi, body) in b.bodies.iter().enumerate() {
                    let c = b.bots.iter().flatten().find(|c| c.body == bi);
                    let fi = s.bot_fighter[bi];
                    eprintln!(
                        "  {} at {:?} dead {} hp {:?} move {:?} perceives {:?} closest {:?} follow {}",
                        body.name,
                        (s.movers[fi].location.x as i32, s.movers[fi].location.y as i32, s.movers[fi].location.z as i32),
                        body.is_dead,
                        s.combat.fighters[fi].health,
                        c.and_then(|c| c.move_request.map(|m| (m.dest.x as i32, m.dest.y as i32))),
                        c.and_then(|c| c.blackboard.get("bPerceivesEnemy").copied()),
                        c.and_then(|c| c.closest_enemy_override),
                        s.bot_follow.get(bi).map(|f| f.path.len()).unwrap_or(0)
                    );
                }
            }
        }
        let m = s.mode.as_ref().unwrap();
        let wave = m.gm.horde().wave;
        if wave > max_wave {
            max_wave = wave;
            eprintln!("t={:.1}s wave {wave}: spawned {spawned}, player kills {kills_by_players}", frame as f32 * dt);
        }
        if wave >= target_wave || m.gm.match_state != mh_mode::game_mode::MatchState::InProgress {
            break;
        }
    }
    let m = s.mode.as_ref().unwrap();
    eprintln!(
        "HRD_Camp (cooked navmesh {cooked}): wave {}, state {:?}, enemies spawned {spawned}, killed by players {kills_by_players}, weapon substitutions {subs}, spawns moved onto the navmesh {moved_spawns}, mode events {counts:?}",
        m.gm.horde().wave,
        m.gm.match_state
    );
    eprintln!("hits {hit_pairs:?}
combat events {kinds:?}");
    if std::env::var("MH_HORDE_DEBUG").is_ok() {
        eprintln!("attacks {attack_geo:#?}");
        eprintln!("motions {motions:?}");
    }
    assert!(spawned > 0, "the waves spawned enemies");
    assert!(kills_by_players > 0, "the players' bots killed enemies");
    assert!(m.gm.horde().wave >= target_wave, "waves 1-{target_wave} were played (reached {})", m.gm.horde().wave);
}

/// repro (ignored): one pawn on HRD_Camp's landscape at the spot where a horde enemy stalled (velocity and
/// acceleration nonzero, location frozen, Walking on Landscape_0.LandscapeHeightfieldCollisionComponent_234, normal z
/// 0.93), forward input toward the next navmesh corner for 3 s
#[test]
#[ignore]
fn repro_landscape_stall() {
    let Ok(vfs) = Vfs::mount_default().map(Arc::new) else { return eprintln!("SKIP: no paks") };
    let Ok(matrix) = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) else { return eprintln!("SKIP: no spec") };
    let Ok(rec) = std::fs::read_to_string(repo().join("core/tests/golden/character/records.json")) else { return eprintln!("SKIP: no records") };
    let records = RecordsJson(&rec).load().unwrap();
    let pkg = vfs.list().find(|p| p.to_lowercase().ends_with("/hrd_camp.umap")).unwrap().trim_end_matches(".umap").to_string();
    let pk = Pkgs::new(Reader::new(vfs.clone()));
    let d = read(&pk, &pkg);
    let src = mh_assets::pak_source::PakSource::new(vfs.clone());
    let tri = |p: &str| -> Option<Vec<[f64; 3]>> {
        let m = mh_assets::static_mesh::lod0(&src, p).ok()?;
        Some(m.indices.iter().map(|&i| m.vertices.positions[i as usize].map(|c| c as f64)).collect())
    };
    let w = CollisionWorld::build(&pk, &d, Some(&tri));
    {
        use mh_character::world::World;
        let st = FVector::new(-5631.4624, 7713.6226, 728.1432);
        for (r, hh) in [(42.0f32, 96.0f32), (34.0, 88.0), (42.0, 90.0)] {
            eprintln!("overlap r {r} hh {hh}: {}", w.overlap_capsule(st, r, hh));
        }
    }
    let ld = mh_sim::load::load(&matrix, vfs.clone(), &[LS]).expect("load");
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), records, Box::new(w), 1.0 / 60.0);
    let sp = d.spawns[0].xf.translation();
    let start = FVector::new(-5631.4624, 7713.6226, 728.1432);
    let fi = s.add_fighter(&FighterDesc { name: "X".into(), weapon: LS.into(), location: start, yaw: 44.6, ..Default::default() });
    let ctrl = s.add_fighter(&FighterDesc { name: "C".into(), weapon: LS.into(), location: FVector::new(sp[0] as f32, sp[1] as f32, sp[2] as f32 + 20.0), yaw: 0.0, ..Default::default() });
    for f in 0..180 {
        let inp = mh_sim::SimInput { fwd: 1.0, yaw: Some(44.6), ..Default::default() };
        let inc = mh_sim::SimInput { fwd: 1.0, yaw: Some(0.0), ..Default::default() };
        s.step(&[(fi, inp), (ctrl, inc)]);
        if f % 30 == 0 {
            let m = &s.movers[fi];
            eprintln!("t={:.2} at {:?} vel {:?} mode {:?}", f as f32 / 60.0, m.location, m.velocity, m.mode);
        }
    }
    eprintln!("control pawn at a player start: {:?} vel {:?}", s.movers[ctrl].location, s.movers[ctrl].velocity);
    let moved = (s.movers[fi].location - start).length();
    eprintln!("moved {moved:.1} cm in 3 s");
}

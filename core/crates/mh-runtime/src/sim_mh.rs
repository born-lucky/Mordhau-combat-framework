//! sim_mh.rs - `SimBackend` over rust-combat's mh-sim facade (core/crates/mh-sim, docs/RUST_CORE.md "mh-sim"):
//! exe-exact combat (mordhau-core) + mh-character ExeMovement against the map's collision (rust-pak's
//! mh_level::collision::CollisionWorld, built here from the same map read) + weapon traces on the posed bodies.
//!
//! Data: the spec matrix (data_gen/spec, mh-spec) + the paks (mh_sim::load) for the combat records and the trace
//! geometry; character movement records from core/tests/golden/character/records.json (the record dump of
//! godot/tools/export_golden_character.gd, mh_character::RecordsJson). That dump is the reference's own record layer
//! (Triternion data, git-ignored); a pak-side CharacterSource is rust-character's. Without either the runtime falls
//! back to the stub (evidence: summary.json sim_note).

use crate::sim::{CurrentWeaponTrace, FighterView, FrameInput, RawCamera1P, SimBackend};
use mh_sim::{FighterDesc, Sim, SimInput};
use mordhau_core::ue::{FTransform, FVector};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

/// One-handed arming sword: the weapon the 1H idle / locomotion clips belong to (BS_1H_Sword_Locomotion_3P_NEW is
/// its FLD_WPN_EQUIP_UPPER_BLEND_SPACE); path as in core/tests/scenarios/combat.json "weapons.ARMING".
pub const DEFAULT_WEAPON: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/OneHanded/BP_ArmingSword";

pub struct MhSim {
    pub matrix: Arc<mh_spec::Spec>,
    pub records_path: std::path::PathBuf,
    pub sim: Option<Sim>,
    /// the first loadout's weapon (idle fallback, evidence)
    pub weapon: String,
    /// every weapon a loadout of this run holds (mh_sim::load builds their records + trace geometry)
    pub weapons: Vec<String>,
    pub next_weapon: Option<String>,
    /// the next spawn's left-hand item and each fighter's (fidelity-audit r5)
    pub next_left: Option<String>,
    pub fighter_lefts: Vec<String>,
    /// the next spawn's wearable classes (EWearableSlot order; rust-character r9: armor movement factors)
    pub next_wearables: Option<Vec<String>>,
    pub fighter_weapons: Vec<String>,
    /// fighter id -> sim fighter index
    pub ids: Vec<usize>,
    pub teams: Vec<Option<i64>>,
    pub events_tail: Vec<String>,
    pub hits: usize,
    pub collision_info: serde_json::Value,
    /// rust-combat's animation stand-in (mh_sim::anim::ClipPoser: the attack clip at rate 1, else the reference pose)
    /// until its fighter_anim / motion_anim port lands in mh-sim; while it shows the reference pose, the weapon's idle
    /// (FLD_WPN_EQUIP_LOWER_ANIMATION, looped) is set instead so the rendered and traced pose stay one pose
    pub poser: Option<mh_sim::anim::ClipPoser>,
    pub idle: Option<Rc<mh_assets::anim::AnimSequence>>,
    pub anim: String,
    /// bots (mh-mode AMordhauAIController + behavior tree, mh-sim Sim::bots): the profile / tree / Kismet literals from
    /// the spec matrix; every fighter gets a BotBody (perception), only bots get a controller
    pub bot_data: Option<BotData>,
    pub bot_ids: Vec<u32>,
    pub bot_note: Option<String>,
    pub golden: bool,
    pub records_src: String,
    pub repo: std::path::PathBuf,
    pub out_events: Vec<serde_json::Value>,
    pub out_tracers: Vec<crate::sim::TracerSegment>,
    raw_cameras_1p: HashMap<u32, RawCamera1P>,
    pub cam_world: Option<Arc<mh_level::collision::CollisionWorld>>,
    /// Original cooked weapon-query owner for this map, separate from movement collision.
    pub complex_world: Option<Rc<mh_level::complex::ComplexTraceWorld>>,
    /// floor body -> EPhysicalSurface (footsteps), and the paks to read the physical materials
    pub surfaces: HashMap<u32, usize>,
    pub vfs: Option<Arc<mh_pak::Vfs>>,
    /// step with the caller's frame dt (Sim::step_dt) instead of the fixed step
    pub variable_dt: bool,
    /// the session's rand() seed (sim.rs session_seed)
    pub rand_seed: u32,
    timing_base: Option<Rc<mordhau_core::data::Spec>>,
    timing_pending: Option<Rc<mordhau_core::data::Spec>>,
    timing_revision: u64,
    timing_catalog_epoch: u64,
}

pub struct BotData {
    /// the game's bot roster (BP_MordhauSingleton BotProfiles: mh_mode::ai::bot_profiles; rust-mode-ai r6): BeginPlay
    /// picks a BOT_* class, its BOTBEHAVIOR_* profile and loadout. None -> `profile` below for every bot.
    pub roster: Option<mh_mode::ai::bot_profiles::BotRoster>,
    /// the spec matrix's BOTBEHAVIOR_* records by class name (ENT_BOT_*)
    pub profiles: std::collections::HashMap<String, mh_mode::ai::Profile>,
    pub profile: mh_mode::ai::Profile,
    pub tree: mh_mode::ai::TreeDef,
    pub kismet: std::sync::Arc<mh_mode::kismet::Kismet>,
    pub profile_name: String,
    pub tree_name: String,
}

/// UBotBehaviorProfile (BOTBEHAVIOR_*) and UBehaviorTree (BT_*) records from the spec matrix (ENT_BOT_* FLD_BOT_<field>,
/// ENT_BT_* FLD_BT_BLACKBOARD / FLD_BT_ROOT: the BotData.Profile / BotData.TreeDef records mh-mode's serde types
/// read). The duel bot: BOTBEHAVIOR_Knight + BT_Deathmatch, the pair the reference's bots_duel golden runs
/// (core/tests/golden/mode/bots_duel.jsonl; every Knight profile field equal to the matrix, checked r4).
pub fn bot_data(matrix: &mh_spec::Spec, vfs: &Arc<mh_pak::Vfs>, repo: &std::path::Path, profile: &str, tree: &str) -> Result<BotData, String> {
    let pe = matrix.entities.get(&format!("ENT_BOT_BOTBEHAVIOR_{profile}")).ok_or(format!("spec: no bot profile {profile}"))?;
    let pj: serde_json::Map<String, serde_json::Value> =
        pe.values.iter().filter_map(|(k, v)| k.strip_prefix("FLD_BOT_").map(|n| (n.to_lowercase(), v.clone()))).collect();
    let profile_rec: mh_mode::ai::Profile = serde_json::from_value(serde_json::Value::Object(pj)).map_err(|e| format!("bot profile: {e}"))?;
    let te = matrix.entities.get(&format!("ENT_BT_{tree}")).ok_or(format!("spec: no behavior tree {tree}"))?;
    let tj = serde_json::json!({"asset": format!("Mordhau/Content/Mordhau/AI/BehaviorTrees/{tree}"),
        "blackboard": te.values.get("FLD_BT_BLACKBOARD"), "root": te.values.get("FLD_BT_ROOT")});
    let (tree_rec, tree_src) = match mh_mode::ai::TreeDef::from_json(&tj.to_string()) {
        Ok(t) => (t, "spec matrix"),
        Err(e) => {
            // fallback (spec bt.json has unserialized nested nodes, reported to the sheets builder r4): the same tree
            // as the reference's bots_duel golden records it (core/tests/golden/mode/bots_duel.jsonl line 2)
            let g = repo.join("core/tests/golden/mode/bots_duel.jsonl");
            let line = std::fs::read_to_string(&g).ok().and_then(|t| t.lines().nth(1).map(String::from)).ok_or(format!("{e}; no {}", g.display()))?;
            let mut h: serde_json::Value = serde_json::from_str(&line).map_err(|x| x.to_string())?;
            mordhau_core::data::decode_exact(&mut h);
            let t: mh_mode::ai::TreeDef = serde_json::from_value(h["bots"][0]["tree"].clone()).map_err(|x| x.to_string())?;
            if !t.asset.ends_with(tree) {
                return Err(format!("{e}; golden tree is {}", t.asset));
            }
            (t, "golden bots_duel (spec bt.json unparsable)")
        }
    };
    let kj = mh_host::Data { matrix: matrix.clone(), vfs: vfs.clone() }.kismet_json()?;
    let kismet = std::sync::Arc::new(mh_mode::kismet::Kismet::from_json(&kj)?);
    // rust-mode-ai r6: the roster BeginPlay draws from, and every behaviour profile it can name
    let rd = mh_pak::Reader::new(vfs.clone());
    let load = |p: &str| rd.read(p);
    let roster = mh_mode::ai::bot_profiles::BotRoster::from_exports(&load).ok();
    let mut profiles = std::collections::HashMap::new();
    for id in matrix.entities.keys().filter_map(|k| k.strip_prefix("ENT_BOT_")) {
        if let Ok(p) = mh_mode::spec_bots::profile(matrix, id) {
            profiles.insert(id.to_string(), p);
        }
    }
    Ok(BotData { roster, profiles, profile: profile_rec, tree: tree_rec, kismet, profile_name: profile.into(), tree_name: format!("{tree} ({tree_src})") })
}

impl MhSim {
    pub fn new(matrix: Arc<mh_spec::Spec>, repo: &std::path::Path, golden: bool, weapons: Vec<String>) -> Result<MhSim, String> {
        let records_path = repo.join("core/tests/golden/character/records.json");
        if golden && !records_path.is_file() {
            return Err(format!("no character records {}", records_path.display()));
        }
        Ok(MhSim {
            matrix,
            records_path,
            sim: None,
            weapon: weapons.first().cloned().unwrap_or_else(|| DEFAULT_WEAPON.into()),
            weapons: if weapons.is_empty() { vec![DEFAULT_WEAPON.into()] } else { weapons },
            next_weapon: None,
            next_left: None,
            fighter_lefts: Vec::new(),
            next_wearables: None,
            fighter_weapons: Vec::new(),
            ids: Vec::new(),
            teams: Vec::new(),
            events_tail: Vec::new(),
            hits: 0,
            collision_info: serde_json::Value::Null,
            poser: None,
            idle: None,
            anim: String::new(),
            bot_data: None,
            bot_ids: Vec::new(),
            bot_note: None,
            golden,
            records_src: String::new(),
            repo: repo.to_path_buf(),
            out_events: Vec::new(),
            out_tracers: Vec::new(),
            raw_cameras_1p: HashMap::new(),
            cam_world: None,
            complex_world: None,
            surfaces: HashMap::new(),
            vfs: None,
            variable_dt: false,
            rand_seed: 1,
            timing_base: None,
            timing_pending: None,
            timing_revision: 0,
            timing_catalog_epoch: 0,
        })
    }
}

impl MhSim {
    fn flush_timing_edit(&mut self) -> bool {
        let Some(s) = &mut self.sim else { return false };
        if !s.combat.timing_edit_ready() { return false; }
        let Some(spec) = self.timing_pending.take() else { return true };
        s.combat.install_timing_spec(spec).expect("idle validated timing install");
        self.timing_revision += 1;
        true
    }
    /// A BotBody for every fighter (bot perception sees players too; mh-sim tests/sim.rs bots_fight_with_real_traces)
    fn add_body(&mut self, fi: usize, yaw: f32) {
        let (Some(s), Some(bd)) = (&mut self.sim, &self.bot_data) else { return };
        let seed = self.rand_seed;
        let mut bots = s.bots.take().unwrap_or_else(|| mh_mode::ai::Bots { k: bd.kismet.clone(), rng: mordhau_core::ue::CrtRand::new(seed), ..Default::default() });
        let mut body = mh_mode::ai::BotBody::new(&s.combat.fighters[fi].name);
        body.location = s.movers[fi].location;
        body.yaw = yaw as f64;
        body.weapon_length = s.combat.fighters[fi].tracer.length;
        body.max_walk_speed = s.records.movement.max_walk_speed;
        body.pawn = mh_mode::ai::combat_host::pawn_view(&s.combat, fi);
        body.weapon = mh_mode::ai::combat_host::weapon_view(&s.combat, fi);
        body.inventory = vec![mh_mode::ai::InventoryItem { weapon: body.weapon.clone(), is_fists: false, ranged: false }];
        // APawn BaseEyeHeight (UE 4.26 APawn ctor 64; Mordhau's CDOs do not override it: UNCONFIRMED): the sight
        // sense's eye (mh-mode controller.rs sight_stimulus)
        body.eye_height = 64.0;
        bots.add_body(body);
        s.bot_fighter.push(fi);
        s.bots = Some(bots);
    }
}

impl SimBackend for MhSim {
    fn name(&self) -> &'static str {
        "mh-sim"
    }
    fn world_generation(&self) -> u64 { self.timing_catalog_epoch }

    fn attach_level(&mut self, vfs: Option<&Arc<mh_pak::Vfs>>, map: &str) -> Result<(), String> {
        let vfs = vfs.ok_or("mh-sim needs the paks")?.clone();
        self.vfs = Some(vfs.clone());
        self.surfaces.clear();
        // the bot roster's loadout weapons (rust-mode-ai r6): BeginPlay may give a bot any BotCharacterProfiles
        // loadout, so its first weapon is loaded with the session's when the spec has its records
        {
            let rd = mh_pak::Reader::new(vfs.clone());
            let load = |p: &str| rd.read(p);
            if let Ok(r) = mh_mode::ai::bot_profiles::BotRoster::from_exports(&load) {
                for (w, _) in &r.loadouts {
                    if let Some(w) = w.first() {
                        let id = format!("ENT_WPN_{}", w.rsplit('/').next().unwrap_or(w));
                        if self.matrix.entities.contains_key(&id) && !self.weapons.contains(w) {
                            self.weapons.push(w.clone());
                        }
                    }
                }
            }
        }
        let ws: Vec<&str> = self.weapons.iter().map(|s| s.as_str()).collect();
        let loaded = mh_sim::load::load(&self.matrix, vfs.clone(), &ws)?;
        // movement records: the spec matrix through mh-host (mh-character SpecCharacter, equal to the golden dump as
        // f32: mh-host tests/host.rs); `--golden` = the reference's dump core/tests/golden/character/records.json
        let records = if self.golden {
            let txt = std::fs::read_to_string(&self.records_path).map_err(|e| e.to_string())?;
            self.records_src = "golden records.json".into();
            mh_character::CharacterSource::load(&mh_character::RecordsJson(&txt))?
        } else {
            self.records_src = "mh-host spec matrix".into();
            mh_host::Data { matrix: (*self.matrix).clone(), vfs: vfs.clone() }.character_records()?
        };
        // collision: mh-level's reference world for the map, complex-as-simple meshes from mh-assets LOD0 triangles
        // (core/crates/mh-level/tests/collision.rs recipe)
        let t0 = std::time::Instant::now();
        let pk = mh_level::Pkgs::new(mh_pak::Reader::new(vfs.clone()));
        // the combat test level (level.rs TEST_LEVEL): no map, a flat floor only (first-person r1)
        let is_test = map == crate::level::TEST_LEVEL;
        let d = if is_test { mh_level::LevelData::default() } else { mh_level::read(&pk, map) };
        let src = mh_assets::pak_source::PakSource::new(vfs.clone());
        self.poser = Some(mh_sim::anim::ClipPoser::new(mh_assets::pak_source::PakSource::new(vfs.clone())));
        let idle_path = self
            .matrix
            .entities
            .get(&crate::specdata::SpecData::ent_of("ENT_WPN_", &self.weapon))
            .and_then(|e| e.values.get("FLD_WPN_EQUIP_LOWER_ANIMATION"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        self.idle = mh_assets::anim::decode(&src, &idle_path).ok().map(Rc::new);
        let tris = |pkg: &str| -> Option<Vec<[f64; 3]>> {
            let m = mh_assets::static_mesh::lod0(&src, pkg).ok()?;
            Some(m.indices.iter().map(|&i| m.vertices.positions[i as usize].map(|c| c as f64)).collect())
        };
        let build = || if is_test { mh_level::collision::CollisionWorld::flat_floor(&vfs, 5000.0) } else { mh_level::collision::CollisionWorld::build(&pk, &d, Some(&tris)) };
        let w = build();
        // a second copy for the camera's queries (the sim owns the first as its movement world)
        let cw = Arc::new(build());
        self.cam_world = Some(cw.clone());
        self.collision_info = serde_json::json!({"bodies": w.bodies.len(), "blocking_pawn": w.blocks.iter().filter(|b| **b).count(),
            "shapes": w.shapes.len(), "kill_z": w.kill_z, "gravity_z": w.gravity_z, "damage_volumes": w.damage_volumes.len(),
            "build_secs": t0.elapsed().as_secs_f32()});
        // the bots' navigation (rust-mode-ai r6): the map's cooked RecastNavMesh (mh-nav detour.rs), else the voxel
        // stand-in over this collision. Without it BT_Deathmatch's patrol (FindRandomLocation: a reachable point near
        // the enemies' centroid) fails and a bot that perceives nobody yet (sight 2600 cm) never moves
        let bounds: Vec<mh_nav::Bounds> = d
            .volumes
            .iter()
            .filter(|v| v.class.contains("NavMeshBoundsVolume"))
            .map(|v| mh_nav::Bounds { min: v.min.map(|c| c as f32), max: v.max.map(|c| c as f32) })
            .collect();
        let nav: Box<dyn mh_mode::ai::NavQueries> = if is_test {
            let b = [mh_nav::Bounds { min: [-1500.0, -1500.0, -100.0], max: [1500.0, 1500.0, 300.0] }];
            Box::new(mh_nav::NavMesh::build(&w, &b, mh_nav::NavAgent::mordhau(), mh_nav::BuildOptions::default()))
        } else {
            mh_nav::cooked::nav_for_map(&pk, map, &w, &bounds)
        };
        let mut sim = Sim::new(Rc::new(loaded.spec), Rc::new(loaded.geo), records, Box::new(w), (1.0 / crate::sim::TICK_HZ) as f32);
        sim.nav = Some(nav);
        // footstep / impact surfaces: the hit body's EPhysicalSurface at the point (mh-level surface_at, mh-world r2;
        // mh-sim Sim::surface_of, rust-combat)
        let sw = cw.clone();
        sim.surface_of = Some(Box::new(move |body, p| sw.surface_at(body, [p.x as f64, p.y as f64, p.z as f64])));
        // the bots' hearing: the noise-making character sounds' loudness (mh-sim noise.rs, rust-mode-ai r6)
        sim.noise_loudness = mh_sim::noise::NoiseLoudness::read_with_weapons(&mh_pak::Reader::new(vfs.clone()), &ws);
        // rust-combat's animation graph port (mh-sim animgraph.rs: montages, windup/release positions, AutoBlend,
        // slots, LowerBack LayeredBoneBlend over the weapon LowerAnimation loop): every fighter gets PoseSource::Graph;
        // the ClipPoser stand-in and the idle fallback are then off
        match mh_sim::animgraph::AnimAssets::new(vfs.clone()) {
            Ok(a) => {
                sim.enable_anim(a);
                self.poser = None;
                self.idle = None;
                self.anim = "mh-sim animgraph".into();
            }
            Err(e) => self.anim = format!("ClipPoser stand-in (animgraph unavailable: {e})"),
        }
        let mut baseline = (*sim.combat.spec).clone();
        if let Some(a) = &sim.anim {
            // The core catalog contains TP curves. Also expose the original selected FP combo
            // assets for explicit key editing without changing baseline perspective playback.
            let additional: Vec<_> = baseline.motion_defs.iter().filter(|(_,m)| m.attack.is_some())
                .filter_map(|(p,_)| {
                    let path = a.persp(p, "ComboWindUpCurve", true);
                    a.curve(&path).map(|(keys,pre,post)| (path,mordhau_core::data::Curve { keys,pre,post }))
                }).collect();
            for (path,curve) in additional { baseline.curves.entry(path).or_insert(curve); }
        }
        self.timing_base = Some(Rc::new(baseline));
        self.timing_pending = None;
        self.timing_revision = 0;
        self.timing_catalog_epoch += 1;
        let rd = mh_pak::Reader::new(vfs.clone());
        let physics_dlls = mh_pak::game_dir().join("Engine/Binaries/ThirdParty/PhysX3/Win64/VS2015");
        match mh_sim::ragdoll::Ragdolls::open(&rd, &self.repo.join("state/physics/mh_physx.dll"), &physics_dlls, cw.gravity_z as f32) {
            Ok(mut r) => {
                if let Some(a) = &sim.anim {
                    // Enumerate the pure native death chooser without advancing the game's rand stream.
                    // Load its finite clips/notifies during level preparation, never on the first kill.
                    let mut sequences = std::collections::BTreeSet::new();
                    for angle in [-180., -90., 0., 90., 180.] {
                        for random in 0..6 {
                            for bone in ["", "Spine"] {
                                for subtype in [0, 2] {
                                    sequences.insert(mh_sim::ragdoll::death_sequence(angle, random, bone, 1, subtype));
                                }
                            }
                        }
                    }
                    for sequence in sequences {
                        let _ = a.clip(&sequence);
                        let _ = a.notifies(&sequence);
                    }
                }
                for weapon in &ws {
                    let d=rd.defaults(weapon);
                    r.force_multipliers.insert(weapon.to_string(),d.get("RagdollForceMultiplier").and_then(|v|v.as_f64()).unwrap_or(3.5) as f32);
                    if d.get("bForceRagdollOnDeath").and_then(|v|v.as_bool())==Some(true) {r.force_ragdoll.insert(weapon.to_string());}
                }
                for shape in &cw.shapes {
                    let body = &cw.bodies[shape.body as usize];
                    if !body.collision_enabled.contains("Physics") || body.response("PhysicsBody") != mh_level::collision::Resp::Block { continue; }
                    let pts: Vec<_> = shape.pts.iter().map(|p|p.map(|c|c as f32)).collect();
                    let small = (0..3).map(|axis|(shape.max[axis]-shape.min[axis]) as f32*0.5).fold(f32::INFINITY,f32::min);
                    match r.scene.add_static(&pts,shape.radius as f32,(small*0.01).clamp(0.0001,1.)) {
                        Ok(())=>r.world_shapes+=1,
                        Err(e)=>{r.skipped_world_shapes+=1; if r.errors.len()<10 { r.errors.push(format!("{}: {e}",body.name)); }},
                    }
                }
                r.skipped_world_shapes += cw.heightfields.len();
                // World cooking warnings remain diagnostic; they must not suppress valid corpse bodies.
                for e in &r.errors { bevy::log::warn!("corpse world collision: {e}"); }
                r.errors.clear();
                self.collision_info["corpse_world_shapes"] = serde_json::json!(r.world_shapes);
                self.collision_info["corpse_skipped_shapes"] = serde_json::json!(r.skipped_world_shapes);
                sim.ragdolls = Some(r);
            }
            Err(e) => { self.collision_info["corpse_error"] = serde_json::json!(e); bevy::log::warn!("corpse physics unavailable: {e}"); }
        }
        // A fresh sim starts with no old-map callback. Both gameplay and presentation use this owner.
        sim.set_world_static_query(None);
        let mut complex_world = None;
        if is_test {
            self.collision_info["weapon_complex"] = serde_json::json!({"available":false,
                "reason":"TestLevel has no original map cooked triangles"});
        } else {
            let provider = mh_physics::Scene::open(&self.repo.join("state/physics/mh_physx.dll"),
                &physics_dlls,cw.gravity_z as f32,0.7,0.3,100.,1000.)
                .and_then(|scene| mh_level::complex::ComplexTraceWorld::build(&pk,&d,&cw,scene));
            match provider {
                Ok(provider) => {
                    self.collision_info["weapon_complex"] = serde_json::json!({"available":provider.geometry_count()>0,
                        "scope":"original static cooked triangles", "geometries":provider.geometry_count(),
                        "skipped":provider.skipped});
                    let provider = Rc::new(provider);
                    let query_world = provider.clone();
                    sim.set_world_static_query(Some(Rc::new(move |start,end| {
                        match query_world.ray([start.x,start.y,start.z],[end.x,end.y,end.z]) {
                            Ok(Some(hit)) => Some(mordhau_core::combat::world::WorldBlock {
                                impact_point:FVector::new(hit.position[0],hit.position[1],hit.position[2]),
                                actor:true,can_be_damaged:hit.can_be_damaged,component:Some(hit.component),surface:hit.surface,
                            }),
                            Ok(None) => None,
                            Err(error) => panic!("Original complex WorldStatic query failed: {error}"),
                        }
                    })));
                    complex_world = Some(provider);
                },
                Err(error) => {
                    self.collision_info["weapon_complex"] = serde_json::json!({"available":false,"error":error});
                    bevy::log::warn!("original cooked weapon collision unavailable: {error}");
                },
            }
        }
        match bot_data(&self.matrix, &vfs, &self.repo, "Knight", "BT_Deathmatch") {
            Ok(b) => self.bot_data = Some(b),
            Err(e) => self.bot_note = Some(e),
        }
        self.bot_ids.clear();
        self.sim = Some(sim);
        self.complex_world = complex_world;
        // These buffers and id-indexed equipment caches belong to the replaced world.
        // Retained old sweeps must not be drained back into the new world's tracer renderer.
        self.out_tracers.clear();
        self.out_events.clear();
        self.raw_cameras_1p.clear();
        self.fighter_weapons.clear();
        self.fighter_lefts.clear();
        self.ids.clear();
        self.teams.clear();
        Ok(())
    }

    fn spawn(&mut self, loc: [f32; 3], yaw: f32, team: Option<i64>) -> u32 {
        let id = self.ids.len() as u32;
        if let Some(s) = &mut self.sim {
            let w = self.next_weapon.take().unwrap_or_else(|| self.weapon.clone());
            self.fighter_weapons.push(w.clone());
            let left = self.next_left.take().filter(|l| self.weapons.contains(l)).unwrap_or_default();
            self.fighter_lefts.push(left.clone());
            let fi = s.add_fighter(&FighterDesc {
                name: format!("F{id}"),
                weapon: w.clone(),
                // the profile's left-hand item (loadout::hands; UEquipmentSystemComponent::PickUpToSlot rva=0x14d0490)
                left,
                // AMordhauPlayerState Team 255 = none (mordhau-core Fighter.team)
                team: team.unwrap_or(255),
                location: FVector::new(loc[0], loc[1], loc[2]),
                yaw,
            });
            // offline play is a standalone game: no character's anim instance is a dedicated server's (IsDedicatedServer 0,
            // client AnimLOD; NativeUpdateAnimation rva=0x1501930 decomp 500-506) (fidelity-audit r6; mh-net's server and
            // the parity probes keep mh-sim's dedicated-server default)
            s.set_server_pose(fi, false);
            // the movement gear (rust-character r9): the weapon's AMordhauEquipment movement fields and the Head /
            // UpperChest / Legs pieces' armor factors from the paks' class defaults (UpdateEquipmentSpeedAndAcceleration
            // rva=0x14dcd30 -> UpdateArmorSpeedAndAcceleration rva=0x14dc0c0)
            let wear = self.next_wearables.take().unwrap_or_default();
            if let Some(v) = &self.vfs {
                let pk = mh_level::Pkgs::new(mh_pak::Reader::new(v.clone()));
                let wd = pk.defaults(&w);
                let em = mh_character::equipment::EquipmentMovement::from_defaults(&w, &serde_json::Value::Object((*wd).clone()));
                let piece = |slot: usize| -> Option<mh_character::equipment::ArmorWearable> {
                    let c = wear.get(slot).filter(|c| !c.is_empty())?;
                    Some(mh_character::equipment::ArmorWearable::from_defaults(&serde_json::Value::Object((*pk.defaults(c)).clone())))
                };
                // EWearableSlot Head 0, UpperChest 2, Legs 7 (WearableObjectInstances[0] / [2] / [7])
                s.set_movement_gear(fi, Some(&em), None, [piece(0), piece(2), piece(7)], &Default::default());
            }
            self.ids.push(fi);
            self.teams.push(team);
            self.add_body(fi, yaw);
        }
        id
    }

    fn spawn_bot(&mut self, loc: [f32; 3], yaw: f32, team: Option<i64>) -> Option<u32> {
        // the game's bot (rust-mode-ai r6): AMordhauAIController::BeginPlay rva=0x14f0ba0 picks a BOT_* profile from
        // the singleton's roster on the world rand() stream; its loadout's first weapon is the pawn's when this
        // session loaded it (else the session weapon: UNCONFIRMED stand-in, noted in the status)
        let preview = match (&self.sim, &self.bot_data) {
            (Some(s), Some(bd)) => bd.roster.as_ref().and_then(|r| s.bots.as_ref().map(|b| b.preview_bot_choice(r)).unwrap_or_else(|| {
                mh_mode::ai::Bots { k: bd.kismet.clone(), rng: mordhau_core::ue::CrtRand::new(self.rand_seed), ..Default::default() }.preview_bot_choice(r)
            })),
            _ => None,
        };
        if let Some(c) = &preview {
            match c.weapons.first().filter(|w| self.weapons.contains(w)) {
                Some(w) => self.next_weapon = Some(w.clone()),
                None => self.bot_note = Some(format!("{}: loadout {:?} not loaded this session; using {}", c.name, c.weapons, self.weapon)),
            }
        }
        let id = self.spawn(loc, yaw, team);
        let (Some(s), Some(bd)) = (&mut self.sim, &self.bot_data) else { return None };
        let fi = *self.ids.get(id as usize)?;
        let body = s.bot_fighter.iter().position(|&f| f == fi)?;
        let now = s.combat.now;
        let mut bots = s.bots.take()?;
        let mut host = mh_mode::ai::combat_host::CombatHost { world: &mut s.combat, fighter_of: s.bot_fighter.clone() };
        let profiles = &bd.profiles;
        let profile_of = |n: &str| if n.is_empty() { Some(mh_mode::ai::Profile::default()) } else { profiles.get(n).cloned() };
        let chosen = match &bd.roster {
            Some(r) => bots.add_bot_from_roster(body, r, &profile_of, Some(&bd.tree), now, &mut host).map(|x| x.1),
            None => None,
        };
        if chosen.is_none() {
            bots.add_bot(body, bd.profile.clone(), Some(&bd.tree), now, &mut host);
        }
        s.bots = Some(bots);
        if let Some(c) = chosen {
            let w = self.fighter_weapons.get(id as usize).cloned().unwrap_or_default();
            self.bot_note = Some(format!("bot {id}: {} ({}), loadout {:?} {:?}, wields {}", c.name, c.behavior, c.loadout, c.weapons, w.rsplit('/').next().unwrap_or("")));
        }
        self.bot_ids.push(id);
        Some(id)
    }

    fn collision(&self) -> Option<Arc<mh_level::collision::CollisionWorld>> {
        self.cam_world.clone()
    }
    fn preview_bot_loadout(&self) -> Option<usize> {
        let (s, bd) = (self.sim.as_ref()?, self.bot_data.as_ref()?);
        let r = bd.roster.as_ref()?;
        let c = match s.bots.as_ref() {
            Some(b) => b.preview_bot_choice(r),
            None => mh_mode::ai::Bots { k: bd.kismet.clone(), rng: mordhau_core::ue::CrtRand::new(self.rand_seed), ..Default::default() }.preview_bot_choice(r),
        };
        c?.loadout
    }
    fn set_rand_seed(&mut self, seed: u32) {
        self.rand_seed = seed;
        if let Some(b) = self.sim.as_mut().and_then(|s| s.bots.as_mut()) {
            b.rng = mordhau_core::ue::CrtRand::new(seed);
        }
    }
    fn set_variable_dt(&mut self, on: bool) -> bool {
        self.variable_dt = on;
        true
    }
    fn tick(&mut self, dt: f32, inputs: &HashMap<u32, FrameInput>) {
        self.flush_timing_edit();
        let Some(s) = &mut self.sim else { return };
        let bot_in = s.bot_inputs();
        let mut v: Vec<(usize, SimInput)> = inputs
            .iter()
            .filter_map(|(id, i)| {
                let fi = *self.ids.get(*id as usize)?;
                Some((fi, SimInput {
                    fwd: i.fwd,
                    right: i.right,
                    jump: i.jump,
                    sprint: i.sprint,
                    crouch: i.crouch,
                    yaw: i.yaw,
                    look_up: i.look_up,
                    attack: i.attack,
                    feint: i.feint,
                    parry: i.parry,
                    controller_angling_x: i.controller_angling_x,
                    release_block: i.release_block,
                    switch_mode: i.switch_mode,
                    toggle_mode: i.toggle_mode,
                    fire: None, // rust-combat r5: mh-sim SimInput.fire (ranged)
                    // AAdvancedCharacter::Turn rva=0x14a8a90 / LookUp rva=0x1489930 are owning-client input functions:
                    // input.rs already applied the TurnCaps clamp and its per-frame refill (LODTick rva=0x14887b0), so
                    // the sim takes the player's control rotation as given (fidelity-audit r3; bots keep the sim cap)
                    turn_applied_by_client: !self.bot_ids.contains(id),
                }))
            })
            .collect();
        // bot fighters take their controller's input (Sim::bot_inputs)
        v.retain(|(fi, _)| !bot_in.iter().any(|(b, _)| b == fi));
        v.extend(bot_in);
        // poses for this step (mh-sim frame order 4: the pose the traces run against = the pose rendered)
        if let Some(p) = &mut self.poser {
            p.pose(s);
        }
        if let Some(idle) = &self.idle {
            let t = if idle.sequence_length > 0.0 { (s.combat.now as f32 + s.dt) % idle.sequence_length } else { 0.0 };
            for fi in 0..s.poses.len() {
                if matches!(s.poses[fi], mh_sim::PoseSource::Reference) {
                    s.poses[fi] = mh_sim::PoseSource::Host(s.geo.skeleton.sample(idle, t));
                }
            }
        }
        if self.variable_dt {
            s.step_dt(&v, dt);
        } else {
            s.step(&v);
        }
        for t in s.sampled_traces.borrow_mut().drain(..) {
            if let Some(id) = self.ids.iter().position(|&fi| fi == t.fighter) {
                self.out_tracers.push(crate::sim::TracerSegment { fighter: id as u32, tick: t.tick,
                    start: [t.start.x, t.start.y, t.start.z], end: [t.end.x, t.end.y, t.end.z], environment_only: t.environment_only });
            }
        }
        let (ev, log) = s.drain();
        {
            let posed = s.posed.borrow();
            for e in &ev {
                let mut e = serde_json::Value::Object(e.clone());
                let tracer_of = e.get("attacker").or_else(|| if e.get("kind").and_then(|k| k.as_str()) == Some("world_hit") { e.get("who") } else { None });
                if let Some(an) = tracer_of.and_then(|a| a.as_str()).map(String::from) {
                    if let Some(p) = posed.get(&an) {
                        let t = p.tracer.cur_end;
                        e["pos_ue"] = serde_json::json!([t.x, t.y, t.z]);
                        // AMordhauWeapon::PrepareForTracing rva=0x16378e0 (decomp AMordhauWeapon.cpp 3644-3667):
                        // LastObservedTraceDirection = GetSafeNormal((CurStart - PrevStart) + (CurEnd - PrevEnd))
                        let tr = &p.tracer;
                        let d = [(tr.cur_start.x - tr.prev_start.x) + (tr.cur_end.x - tr.prev_end.x),
                            (tr.cur_start.y - tr.prev_start.y) + (tr.cur_end.y - tr.prev_end.y),
                            (tr.cur_start.z - tr.prev_start.z) + (tr.cur_end.z - tr.prev_end.z)];
                        let l2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
                        let n = if l2 == 1.0 { d } else if l2 < 1e-8 { [0.0; 3] } else { let r = 1.0 / l2.sqrt(); [d[0] * r, d[1] * r, d[2] * r] };
                        e["trace_dir"] = serde_json::json!(n);
                    }
                    let aw = s.combat.fighter_index(&an).and_then(|fi| self.ids.iter().position(|&x| x == fi)).and_then(|id| self.fighter_weapons.get(id).cloned());
                    e["attacker_weapon"] = serde_json::json!(aw.unwrap_or_else(|| self.weapon.clone()));
                } else if let Some(w) = e.get("who").or_else(|| e.get("victim")).and_then(|a| a.as_str()).map(String::from) {
                    if let Some(fi) = s.combat.fighter_index(&w) {
                        let l = s.movers[fi].location;
                        e["pos_ue"] = serde_json::json!([l.x, l.y, l.z]);
                    }
                }
                self.out_events.push(e);
            }
        }
        self.hits += ev.iter().filter(|e| e.get("kind").and_then(|t| t.as_str()) == Some("hit")).count();
        self.events_tail.extend(log);
        let n = self.events_tail.len();
        if n > 400 {
            self.events_tail.drain(..n - 400);
        }
    }

    fn ticks(&self) -> u64 {
        self.sim.as_ref().map(|s| s.combat.tick_n.max(0) as u64).unwrap_or(0)
    }

    fn fighters(&self) -> Vec<FighterView> {
        let Some(s) = &self.sim else { return Vec::new() };
        self.ids
            .iter()
            .enumerate()
            .map(|(id, &fi)| {
                let m = &s.movers[fi];
                let f = &s.combat.fighters[fi];
                FighterView {
                    id: id as u32,
                    team: self.teams[id],
                    loc: [m.location.x, m.location.y, m.location.z],
                    vel: [m.velocity.x, m.velocity.y, m.velocity.z],
                    yaw: s.yaw[fi],
                    half_height: m.half_height(),
                    falling: m.mode == mh_character::Mode::Falling,
                    crouched: m.is_crouched,
                    look_up: f.look_up_value as f32,
                    state: crate::sim_core::state_name(&s.combat, fi),
                    health: f.health as f32,
                    stamina: f.stamina,
                }
            })
            .collect()
    }

    fn combat(&self) -> Option<&mordhau_core::combat::World> {
        self.sim.as_ref().map(|s| &s.combat)
    }
    fn export_combat_timings(&self) -> Result<mordhau_core::timing::TimingEdits, String> {
        self.timing_base.as_ref().map(|b| mordhau_core::timing::export(b))
            .ok_or_else(|| "load a map before exporting combat timings".into())
    }
    fn apply_combat_timings(&mut self, edits: Option<&mordhau_core::timing::TimingEdits>) -> Result<bool, String> {
        let base = self.timing_base.as_ref().ok_or("load a map before editing combat timings")?;
        let spec = match edits {
            Some(e) => Rc::new(mordhau_core::timing::apply(base,e)?),
            None => base.clone(),
        };
        self.timing_pending = Some(spec);
        Ok(self.flush_timing_edit())
    }
    fn combat_timing_status(&self) -> serde_json::Value {
        serde_json::json!({"available":self.timing_base.is_some(),"pending_idle":self.timing_pending.is_some(),
            "revision":self.timing_revision,"catalog_epoch":self.timing_catalog_epoch,
            "curve_overrides":self.sim.as_ref().map(|s| s.combat.spec.curve_overrides.len()).unwrap_or(0)})
    }
    fn fighter_index(&self, id: u32) -> Option<usize> {
        self.ids.get(id as usize).copied()
    }
    fn pose_bones(&self) -> Option<Vec<String>> {
        self.sim.as_ref().map(|s| s.geo.skeleton.names.clone())
    }
    fn pose_now(&self, id: u32) -> Option<Vec<FTransform>> {
        let s = self.sim.as_ref()?;
        let fi = *self.ids.get(id as usize)?;
        let name = &s.combat.fighters[fi].name;
        s.posed.borrow().get(name).map(|p| p.bones.clone())
    }
    fn mesh_xf(&self) -> Option<FTransform> {
        self.sim.as_ref().map(|s| s.geo.mesh_xf)
    }
    fn set_pose(&mut self, id: u32, bones: Vec<FTransform>) {
        if let (Some(s), Some(&fi)) = (&mut self.sim, self.ids.get(id as usize)) {
            s.set_pose(fi, bones);
        }
    }
    fn weapon_path(&self, id: u32) -> Option<String> {
        self.fighter_weapons.get(id as usize).cloned()
    }
    fn set_next_wearables(&mut self, classes: &[String]) {
        self.next_wearables = Some(classes.to_vec());
    }
    fn set_next_left(&mut self, left: &str) {
        self.next_left = (!left.is_empty()).then(|| left.to_string());
    }
    fn left_path(&self, id: u32) -> Option<String> {
        self.fighter_lefts.get(id as usize).filter(|l| !l.is_empty()).cloned()
    }
    fn left_world(&self, id: u32) -> Option<FTransform> {
        let (Some(s), Some(&fi)) = (&self.sim, self.ids.get(id as usize)) else { return None };
        s.left_world(fi)
    }
    fn set_next_weapon(&mut self, weapon: &str) {
        if self.weapons.iter().any(|w| w == weapon) {
            self.next_weapon = Some(weapon.to_string());
        }
    }
    /// the body's first physical material (BodySetup / override, mh-level collision) -> UPhysicalMaterial SurfaceType
    /// ("SurfaceType<n>" = EPhysicalSurface n; Mordhau's DefaultEngine.ini names 1 Flesh .. 10 Snow)
    fn floor_surface(&mut self, component: u32) -> usize {
        if let Some(s) = self.surfaces.get(&component) {
            return *s;
        }
        let pm = self.cam_world.as_ref().and_then(|w| w.bodies.get(component as usize)).and_then(|b| b.physical_materials.first().cloned());
        let s = match (pm, &self.vfs) {
            (Some(pm), Some(v)) => {
                let d = mh_pak::Reader::new(v.clone()).defaults(&pm);
                d.get("SurfaceType")
                    .and_then(|x| x.as_str())
                    .and_then(|t| t.rsplit("SurfaceType").next())
                    .and_then(|n| n.parse::<usize>().ok())
                    .unwrap_or(0)
            }
            _ => 0,
        };
        self.surfaces.insert(component, s);
        s
    }
    fn move_by(&mut self, id: u32, offset: [f32; 3]) {
        let Some(&fi) = self.ids.get(id as usize) else { return };
        if let Some(m) = self.sim.as_mut().and_then(|s| s.movers.get_mut(fi)) {
            m.location = FVector::new(m.location.x + offset[0], m.location.y + offset[1], m.location.z + offset[2]);
        }
    }
    fn camera_sweep(&self, start: [f32; 3], end: [f32; 3], radius: f32) -> Option<([f32; 3], bool, String)> {
        let w = self.cam_world.as_ref()?;
        // ObjectTypesToQuery = ECC_WorldStatic (FCollisionObjectQueryParams 1, ComputeCameraPOV line 1015); bodies that
        // answer queries
        let f = |b: u32| w.bodies.get(b as usize).is_some_and(crate::sim::camera_blocker);
        let s3 = start.map(|x| x as f64);
        let e3 = end.map(|x| x as f64);
        let h = w.sweep(s3, e3, radius as f64, radius as f64, &f).into_iter().next()?;
        Some((h.location.map(|x| x as f32), h.start_penetrating, w.bodies[h.body as usize].name.clone()))
    }
    fn camera_channel_sweep(&self, start: [f32; 3], end: [f32; 3], radius: f32) -> Option<(f32, String)> {
        let w = self.cam_world.as_ref()?;
        // SweepSingleByChannel(ECC_Camera): bodies that answer queries and Block the Camera channel (UNCONFIRMED: pawns
        // are not in this world; their Pawn / CharacterMesh profiles ignore Camera)
        let f = |b: u32| w.bodies.get(b as usize).is_some_and(|b| b.queries() && b.response("Camera") == mh_level::collision::Resp::Block);
        let h = w.sweep(start.map(|x| x as f64), end.map(|x| x as f64), radius as f64, radius as f64, &f).into_iter().next()?;
        Some((if h.start_penetrating { 0.0 } else { h.time as f32 }, w.bodies[h.body as usize].name.clone()))
    }
    fn set_camera_collision_offset(&mut self, id: u32, offset: [f32; 3]) {
        if let (Some(s), Some(&fi)) = (&mut self.sim, self.ids.get(id as usize)) {
            s.set_camera_collision_offset(fi, FVector::new(offset[0], offset[1], offset[2]));
        }
    }
    fn respawn(&mut self, id: u32, loc: [f32; 3], yaw: f32) {
        self.raw_cameras_1p.remove(&id);
        let Some(&fi) = self.ids.get(id as usize) else { return };
        if let Some(s) = &mut self.sim {
            s.respawn(fi, FVector::new(loc[0], loc[1], loc[2]), yaw);
        }
    }
    fn set_first_person(&mut self, id: u32, first_person: bool) {
        if let (Some(s), Some(&fi)) = (&mut self.sim, self.ids.get(id as usize)) {
            s.set_first_person(fi, first_person);
        }
    }
    fn set_view_target(&mut self, id: u32, is_view_target: bool, debug_override: bool) {
        if let (Some(s), Some(&fi)) = (&mut self.sim, self.ids.get(id as usize)) {
            s.set_view_target(fi, is_view_target, debug_override);
        }
    }
    fn sprint_fov(&self, id: u32) -> bool {
        let (Some(s), Some(&fi)) = (&self.sim, self.ids.get(id as usize)) else { return false };
        let m = &s.movers[fi];
        m.wants_sprint && (m.sprint_state as u8) > 2
    }
    /// UAttackMotion::TrailWeight (mh-sim Sim::trail_weight, rust-combat: OnTick_Implementation 0x16328c0 decomp
    /// 2879-2890)
    fn trail_weight(&self, id: u32) -> Option<f32> {
        let s = self.sim.as_ref()?;
        Some(s.trail_weight(*self.ids.get(id as usize)?))
    }
    fn alternate_mode(&self, id: u32) -> bool {
        self.sim.as_ref().zip(self.ids.get(id as usize)).is_some_and(|(s, &fi)| s.combat.fighters.get(fi).is_some_and(|f| f.alternate_mode))
    }
    fn weapon_trace_local(&self, id: u32) -> Option<([f32; 3], [f32; 3])> {
        let s = self.sim.as_ref()?;
        let f = s.combat.fighters.get(*self.ids.get(id as usize)?)?;
        f.weapon.as_ref()?;
        let g = s.geo.weapons.get(&f.weapon_path)?;
        let (start, end) = if f.alternate_mode { (g.second_trace_start, g.second_trace_end) } else { (g.trace_start, g.trace_end) };
        let array = |v: Option<FVector>| { let v = v.unwrap_or_default(); [v.x, v.y, v.z] };
        Some((array(start), array(end)))
    }
    fn current_weapon_trace(&self, id: u32) -> Option<CurrentWeaponTrace> {
        let s = self.sim.as_ref()?;
        let f = s.combat.fighters.get(*self.ids.get(id as usize)?)?;
        f.weapon.as_ref()?;
        let g = s.geo.weapons.get(&f.weapon_path)?;
        let (start, end) = if f.alternate_mode {
            (g.second_trace_start?, g.second_trace_end?)
        } else {
            (g.trace_start?, g.trace_end?)
        };
        let posed = s.posed.borrow();
        let p = posed.get(&f.name)?;
        let weapon_world = s.geo.weapon_world(p, g)?;
        let array = |v: FVector| [v.x, v.y, v.z];
        let start_ue_cm = array(weapon_world.apply(start));
        let end_ue_cm = array(weapon_world.apply(end));
        if !start_ue_cm.iter().chain(end_ue_cm.iter()).all(|v| v.is_finite()) { return None; }
        let grip = g.grip_modes.as_ref().map(|m| m[f.alternate_mode as usize].grip_location_local).unwrap_or(g.grip_location_local);
        Some(CurrentWeaponTrace { grip_local_ue_cm: array(grip), start_local_ue_cm: array(start), end_local_ue_cm: array(end), start_ue_cm, end_ue_cm, weapon_world, alternate_mode: f.alternate_mode, owner_equipment: f.right_actor })
    }
    fn first_person(&self, id: u32) -> Option<bool> {
        let s = self.sim.as_ref()?;
        Some(s.fanim.get(*self.ids.get(id as usize)?)?.first_person)
    }
    fn raw_camera_1p(&self, id: u32) -> Option<RawCamera1P> {
        let sample = *self.raw_cameras_1p.get(&id)?;
        (sample.tick == self.ticks() && sample.world_generation == self.world_generation()
            && Some(sample.first_person) == self.first_person(id)).then_some(sample)
    }
    fn clear_raw_cameras_1p(&mut self) { self.raw_cameras_1p.clear(); }
    fn publish_raw_camera_1p(&mut self, id: u32, sample: RawCamera1P) {
        if sample.tick != self.ticks() || sample.world_generation != self.world_generation()
            || Some(sample.first_person) != self.first_person(id)
            || !sample.location_ue_cm.iter().chain(sample.forward_ue.iter()).all(|v| v.is_finite()) {
            self.raw_cameras_1p.remove(&id);
            return;
        }
        self.raw_cameras_1p.insert(id, sample);
    }
    fn trace_sockets(&self, id: u32) -> Option<([f32; 3], [f32; 3])> {
        let s = self.sim.as_ref()?;
        let fi = *self.ids.get(id as usize)?;
        let posed = s.posed.borrow();
        let t = &posed.get(&s.combat.fighters[fi].name)?.tracer;
        Some(([t.cur_start.x, t.cur_start.y, t.cur_start.z], [t.cur_end.x, t.cur_end.y, t.cur_end.z]))
    }
    fn drain_tracers(&mut self) -> Vec<crate::sim::TracerSegment> { std::mem::take(&mut self.out_tracers) }
    fn suicide(&mut self, id: u32) {
        let Some(s) = self.sim.as_mut() else { return };
        let Some(&fi) = self.ids.get(id as usize) else { return };
        if s.combat.fighters[fi].dead {
            return;
        }
        // Suicide rva=0x14a3860: TakeDamage(10000, FDamageEvent, Controller, self) on a living character: the health stat
        // goes to its minimum and OnDied fires (UHealthStatComponent::SetStatValue_Internal, set_health_value)
        let h = s.combat.fighters[fi].health;
        s.combat.set_health_value(fi, h - 10000);
    }
    fn weapon_world(&self, id: u32) -> Option<FTransform> {
        let s = self.sim.as_ref()?;
        let fi = *self.ids.get(id as usize)?;
        let name = &s.combat.fighters[fi].name;
        let posed = s.posed.borrow();
        let p = posed.get(name)?;
        s.geo.weapon_world(p, s.geo.weapons.get(self.fighter_weapons.get(id as usize)?)?)
    }
    fn drain_events(&mut self) -> Vec<serde_json::Value> {
        std::mem::take(&mut self.out_events)
    }
    fn debug(&self) -> serde_json::Value {
        let snap = self.sim.as_ref().map(|s| s.snapshot()).unwrap_or_default();
        let complex_hit = self.complex_world.as_ref().and_then(|world| world.last_hit.borrow().as_ref().map(|hit|
            serde_json::json!({"component":hit.component,"actor_path":hit.actor_path,"can_be_damaged":hit.can_be_damaged,
                "surface":hit.surface,"face":hit.face,"material":hit.material,"distance_ue_cm":hit.distance,
                "position_ue_cm":hit.position,"normal":hit.normal})));
        let equipment: Vec<_> = self.sim.as_ref().map(|s| self.ids.iter().enumerate().map(|(id,&fi)| {
            let f = &s.combat.fighters[fi];
            let actor = f.right_actor.and_then(|a|s.combat.equipment_actor(a));
            serde_json::json!({"fighter":id,"pawn_id":f.id,
                "right_actor":f.right_actor.map(|a|serde_json::json!({"slot":a.slot,"generation":a.generation})),
                "right_actor_state":actor.map(|a|serde_json::json!({"placement":format!("{:?}",a.placement),
                    "b_allow_drop":a.weapon.as_ref().map(|w|w.b_allow_drop)}))})
        }).collect()).unwrap_or_default();
        let attacks: Vec<_> = self.sim.as_ref().map(|s| s.fanim.iter().enumerate().map(|(fi, anim)| {
            let motion = s.combat.cur_m(fi);
            serde_json::json!({"fighter": fi, "motion_bp": motion.map(|m| &m.bp),
                "attack_type": motion.and_then(|m| m.attack()).map(|a| a.ty),
                "is_view_target": s.combat.fighters[fi].is_view_target,
                "first_hit_release_norm": motion.and_then(|m| m.attack()).map(|a| a.first_hit_release_norm),
                "hit_effect_speed_up_exponent": motion.and_then(|m| m.attack()).map(|a| a.ai.hit_effect_speed_up_exponent),
                "animation": anim.ma.attack_debug()})
        }).collect()).unwrap_or_default();
        // evidence (EVD_MOV_013): the anim instance's 1P feel inputs per fighter (upper blend space Direction /
        // Helper_UBVelocity, MovementSpeedScale, GetSpeedFactor(0), SpringPitchYawValue, LookUpValue)
        let feel: Vec<serde_json::Value> = self.sim.as_ref().map(|s| (0..s.fanim.len()).map(|i| serde_json::json!({"upper_input": s.fanim[i].upper_input, "movement_speed_scale": s.fanim[i].movement_speed_scale, "speed_factor": s.movers.get(i).map(|m| m.get_speed_factor(0.0)), "spring": s.fanim[i].proc.spring, "look_up": s.combat.fighters.get(i).map(|f| f.look_up_value), "now": s.combat.now})).collect()).unwrap_or_default();
        serde_json::json!({"fighter_weapons": self.fighter_weapons, "fighter_lefts": self.fighter_lefts,
            "preloaded_weapons": self.weapons, "anim": self.anim, "bots": self.bot_ids, "bot_data": self.bot_data.as_ref().map(|b| format!("{} / {}", b.profile_name, b.tree_name)),
            "bot_note": self.bot_note, "records": self.records_src, "weapon": self.weapon, "collision": self.collision_info, "hits": self.hits,
            "events_tail": self.events_tail, "snapshot": snap, "feel": feel, "attacks": attacks, "equipment":equipment,
            "last_complex_hit":complex_hit})
    }
}

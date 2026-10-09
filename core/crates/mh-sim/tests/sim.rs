//! mh-sim tests on the user's own install (data_gen/spec + the paks; SKIP when either is missing, unless
//! MORDHAU_GOLDEN_REQUIRED=1):
//!  1. spec_matrix_equals_reference_records: the Spec built from the spec matrix + paks (SpecBuilder) holds the same
//!     values as the reference's record dump rounded to f32 (RecordsJsonExe, golden/spec.json) for every record the
//!     golden weapons read (weapons, attacks, equipment, motion defs, curves, character, stats, constants).
//!  2. spec_matrix_runs_the_goldens_identically: every combat golden scenario in exe mode gives bit-identical
//!     snapshots with either Spec.
//!  3. traces_hit_posed_bodies: the UE-space weapon trace on the posed skeleton hits the other character's physics
//!     asset bodies (reference pose, overlapping fighters) and misses one 5 m away; the hit goes through the combat
//!     rules (hit event, flinch).

use mh_character::{BoxWorld, CharacterSource, RecordsJson as CharRecords};
use mh_sim::{FighterDesc, Sim, SimInput};
use mordhau_core::combat::world::Input;
use mordhau_core::combat::World;
use mordhau_core::data::{RecordsJsonExe, SpecSource};
use mordhau_core::ue::FVector;
use serde_json::Value;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

fn core_tests() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests")
}

const LS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";

struct Env {
    matrix: mh_spec::Spec,
    vfs: Arc<mh_pak::Vfs>,
    reference: Option<mordhau_core::data::Spec>,
    weapons: Vec<String>,
}

fn env() -> Option<Env> {
    let required = std::env::var("MORDHAU_GOLDEN_REQUIRED").map(|v| v == "1").unwrap_or(false);
    let matrix = match mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false) {
        Ok(m) => m,
        Err(e) => {
            assert!(!required, "spec matrix: {e:?}");
            eprintln!("SKIP: no spec matrix ({e:?})");
            return None;
        }
    };
    let vfs = match mh_pak::Vfs::mount_default() {
        Ok(v) => Arc::new(v),
        Err(e) => {
            assert!(!required, "paks: {e:?}");
            eprintln!("SKIP: no paks ({e:?})");
            return None;
        }
    };
    let reference = std::fs::read_to_string(core_tests().join("golden/spec.json")).ok().map(|t| RecordsJsonExe(&t).load_spec().unwrap());
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(core_tests().join("scenarios/combat.json")).unwrap()).unwrap();
    let weapons = doc["weapons"].as_object().unwrap().values().map(|v| v.as_str().unwrap().to_string()).collect();
    Some(Env { matrix, vfs, reference, weapons })
}

#[test]
fn spec_matrix_equals_reference_records() {
    let Some(e) = env() else { return };
    let Some(r) = &e.reference else { return eprintln!("SKIP: no golden spec.json") };
    let rd = mh_pak::Reader::new(e.vfs.clone());
    let ws: Vec<&str> = e.weapons.iter().map(|s| s.as_str()).collect();
    let m = mh_sim::spec::SpecBuilder::new(&e.matrix, &rd).build(&ws).expect("SpecBuilder");
    let mut diffs: Vec<String> = Vec::new();
    macro_rules! cmp {
        ($w:expr, $a:expr, $b:expr) => {{
            let (w, a, b): (String, String, String) = ($w, $a, $b);
            if a != b {
                diffs.push(format!("{w}:\n  matrix    {a}\n  reference {b}"));
            }
        }};
    }
    for w in &ws {
        let (a, b) = (m.weapon_setup(w), r.weapon_setup(w));
        cmp!(format!("weapon {w}"), format!("{:?}", a.weapon), format!("{:?}", b.weapon));
        cmp!(format!("equip {w}"), format!("{:?}", a.equip), format!("{:?}", b.equip));
        let mut am: Vec<_> = a.motions.iter().collect();
        let mut bm: Vec<_> = b.motions.iter().collect();
        am.sort();
        bm.sort();
        cmp!(format!("motions {w}"), format!("{am:?}"), format!("{bm:?}"));
        cmp!(format!("tracer {w}"), format!("{:?}", a.tracer), format!("{:?}", b.tracer));
    }
    for (k, d) in &r.motion_defs {
        match m.motion_defs.get(k) {
            Some(x) => {
                // BackpedalSpeedFactor / ShieldWallSpeedFactor: read from the motion Blueprints by the matrix build only
                // (rust-combat r6; the GDScript reference records do not carry them)
                let mut x = (**x).clone();
                x.base.backpedal_speed_factor = d.base.backpedal_speed_factor;
                x.base.shield_wall_speed_factor = d.base.shield_wall_speed_factor;
                cmp!(format!("motion def {k}"), format!("{:?}", x), format!("{:?}", d))
            }
            None => diffs.push(format!("motion def {k}: missing in the matrix build")),
        }
    }
    for (k, c) in &r.curves {
        match m.curves.get(k) {
            Some(x) => cmp!(format!("curve {k}"), format!("{:?}", x), format!("{:?}", c)),
            None => diffs.push(format!("curve {k}: missing")),
        }
    }
    cmp!("character".into(), format!("{:?}", m.character), format!("{:?}", r.character));
    cmp!("constants".into(), format!("{:?}", m.constants), format!("{:?}", r.constants));
    let mut ms: Vec<_> = m.stats.iter().map(|(k, v)| format!("{k}={v:?}")).collect();
    let mut rs: Vec<_> = r.stats.iter().map(|(k, v)| format!("{k}={v:?}")).collect();
    ms.sort();
    rs.sort();
    cmp!("stats".into(), format!("{ms:?}"), format!("{rs:?}"));
    for (k, v) in &r.class_chains {
        cmp!(format!("class chain {k}"), format!("{:?}", m.class_chains.get(k)), format!("{:?}", Some(v)));
    }
    cmp!("block collider".into(), format!("{:?}", m.block_collider), format!("{:?}", r.block_collider));
    for (k, v) in &r.mode_rules {
        cmp!(format!("mode rules {k}"), format!("{:?}", m.mode_rules.get(k)), format!("{:?}", Some(v)));
    }
    for d in &diffs {
        eprintln!("DIFF {d}");
    }
    assert!(diffs.is_empty(), "{} record differences between the spec matrix and the reference dump", diffs.len());
}

fn input_of(ev: &Value, alias: &dyn Fn(&str) -> String) -> Input {
    use mordhau_core::combat::world::Call;
    let s = |k: &str| ev[k].as_str().unwrap_or("").to_string();
    let f = |k: &str, d: f64| ev[k].as_f64().unwrap_or(d);
    let i = |k: &str, d: i64| ev[k].as_f64().map(|x| x as i64).unwrap_or(d);
    let who = s("who");
    match ev["kind"].as_str().unwrap() {
        "attack" => Input::Attack { who, mv: i("move", 0), angle: f("angle", 0.0) },
        "feint" => Input::Feint { who },
        "parry" => Input::Parry { who, bt: i("bt", 0) },
        "release_block" => Input::ReleaseBlock { who },
        "switch_mode" => Input::SwitchMode { who },
        "toggle_mode" => Input::ToggleMode { who },
        "contact" => Input::Contact { who, target: s("target"), bone: ev["bone"].as_str().unwrap_or("Spine1").into() },
        "set" => {
            let v = &ev["value"];
            Input::Call(match s("field").as_str() {
                "stamina" => Call::SetStamina { who, v: v.as_f64().unwrap() as i64 },
                "armor_tier_override" => Call::SetArmorTierOverride { who, v: v.as_f64().unwrap() as i64 },
                "airborne" => Call::SetAirborne { who, v: v.as_bool().unwrap() },
                "look_up_value" => Call::SetLookUp { who, v: v.as_f64().unwrap() },
                x => panic!("set {x}"),
            })
        }
        "weapon_no_drop" => Input::Call(Call::WeaponNoDrop { who }),
        "blocked" => Input::Call(Call::Blocked { who, reason: i("reason", 0), flags: i("flags", 0), time: f("time", 0.0) }),
        "request_parry" => Input::Call(Call::RequestParry { who, bt: i("bt", 0), ftp: ev["ftp"].as_bool().unwrap_or(true) }),
        "preset_attack" => Input::Call(Call::PresetAttack { who, mv: i("move", 0), angle: f("angle", 0.0) }),
        "equip_right" => Input::Call(Call::EquipRight { who, weapon: alias(&s("weapon")) }),
        k => panic!("{k}"),
    }
}

fn run(spec: Rc<mordhau_core::data::Spec>, sc: &Value, alias: &dyn Fn(&str) -> String) -> Vec<Value> {
    let dt = sc["dt"].as_f64().unwrap_or(0.001);
    let mut w = World::new(spec, dt);
    if sc.get("mode").is_some() {
        w.set_game_mode(sc["mode"]["game_mode"].as_str().unwrap(), sc["mode"]["game_state"].as_str().unwrap());
    }
    for fd in sc["fighters"].as_array().unwrap() {
        let fi = w.add_fighter(fd["name"].as_str().unwrap(), &alias(fd["weapon"].as_str().unwrap()), &alias(fd["left"].as_str().unwrap_or("")));
        if let Some(t) = fd["team"].as_i64() {
            w.fighters[fi].team = t;
        }
    }
    for ev in sc["inputs"].as_array().unwrap() {
        w.at(ev["t"].as_f64().unwrap(), input_of(ev, alias));
    }
    let n = (sc["until"].as_f64().unwrap() / dt).round() as i64;
    let mut out = Vec::new();
    while w.tick_n < n {
        w.step();
        out.push(mordhau_core::snapshot::row(&w, 0, 0));
    }
    out
}

#[test]
fn spec_matrix_runs_the_goldens_identically() {
    let Some(e) = env() else { return };
    let Some(r) = e.reference else { return eprintln!("SKIP: no golden spec.json") };
    let rd = mh_pak::Reader::new(e.vfs.clone());
    let ws: Vec<&str> = e.weapons.iter().map(|s| s.as_str()).collect();
    let m = Rc::new(mh_sim::spec::SpecBuilder::new(&e.matrix, &rd).build(&ws).unwrap());
    let r = Rc::new(r);
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(core_tests().join("scenarios/combat.json")).unwrap()).unwrap();
    let aliases = doc["weapons"].clone();
    let alias = move |x: &str| -> String { aliases[x].as_str().unwrap_or(x).to_string() };
    let mut bad = Vec::new();
    for sc in doc["scenarios"].as_array().unwrap() {
        let (a, b) = (run(m.clone(), sc, &alias), run(r.clone(), sc, &alias));
        let k = a.iter().zip(&b).position(|(x, y)| x != y);
        eprintln!("{} {}", if k.is_none() { "SAME" } else { "DIFF" }, sc["name"]);
        if let Some(k) = k {
            bad.push(format!("{} first differs at tick {}", sc["name"], k + 1));
        }
    }
    assert!(bad.is_empty(), "{bad:?}");
}

fn sim(e: &Env, dt: f32) -> Option<Sim> {
    let rec = std::fs::read_to_string(core_tests().join("golden/character/records.json")).ok()?;
    let records = CharRecords(&rec).load().unwrap();
    let ld = mh_sim::load::load(&e.matrix, e.vfs.clone(), &[LS]).expect("load");
    let mut floor = BoxWorld::new();
    floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
    Some(Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), records, Box::new(floor), dt))
}

/// The installed original solver must drive all16 corpse bodies, not an upright pose or a translated capsule.
#[test]
#[cfg(feature = "native-validation")]
#[ignore = "Explicit isolated native validation: reviewed bridge and root dispatch required"]
fn original_physx_death_bodies_fall_and_notify_starts_simulation() {
    eprintln!("PROBE env");
    let e=env().expect("Explicit native diagnostic requires original paks/spec; never skip");
    let bridge=std::path::PathBuf::from(std::env::var_os("MH_PHYSX_VALIDATION_BRIDGE")
        .expect("Explicit reviewed staged DLL path required"));
    assert!(bridge.is_absolute(), "Native diagnostic requires an absolute reviewed bridge path");
    assert!(bridge.is_file(), "Build the original PhysX bridge with scripts/build_physx.py first");
    let rd=mh_pak::Reader::new(e.vfs.clone());
    for (random,immediate) in [(0,true),(5,false)] {
        eprintln!("PROBE sim");
        let mut s=sim(&e,1./120.).unwrap();
        s.dedicated_server=false;
        eprintln!("PROBE anim");
        s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).unwrap());
        let fi=s.add_fighter(&FighterDesc {name:"corpse".into(),weapon:LS.into(),location:FVector::new(0.,0.,96.),..Default::default()});
        eprintln!("PROBE scene");
        let mut r=mh_sim::ragdoll::Ragdolls::open_for_validation(&rd,&bridge,&mh_pak::game_dir().join("Engine/Binaries/ThirdParty/PhysX3/Win64/VS2015"),-980.).unwrap();
        eprintln!("PROBE static");
        r.scene.add_static(&[[-500.,-500.,-20.],[500.,-500.,-20.],[-500.,500.,-20.],[500.,500.,-20.],[-500.,-500.,0.],[500.,-500.,0.],[-500.,500.,0.],[500.,500.,0.]],0.,0.1).unwrap();
        eprintln!("PROBE warm");
        s.ragdolls=Some(r);
        for n in 0..60 {eprintln!("PROBE warm {n}");s.step(&[]);}
        let hips=s.geo.skeleton.find("Hips").unwrap();
        let before={let p=s.posed.borrow();s.geo.bone_world(&p["corpse"],hips).loc.z};
        s.combat.fighters[fi].net.id=random;
        s.combat.set_health_value(fi,0);
        s.step(&[]);
        if immediate {assert_eq!(s.ragdolls.as_ref().unwrap().corpses[0].bodies.len(),16);}
        else {
            assert!(s.ragdolls.as_ref().unwrap().corpses[0].bodies.is_empty());
            let seq=&s.ragdolls.as_ref().unwrap().corpses[0].sequence;
            assert!(seq.ends_with("Death_0"));
            let n=s.anim.as_ref().unwrap().notifies(seq);
            assert!(n.iter().any(|(name,t)|name=="BeginRagdoll" && (*t-0.7990529).abs()<1e-6),"Original notify must be read: {n:?}");
        }
        eprintln!("PROBE initial joint anchors random{random}: {:?}",s.ragdolls.as_ref().unwrap().validation_joint_anchors().unwrap());
        for _ in 0..480 {s.step(&[]);}
        let r=s.ragdolls.as_ref().unwrap();
        assert!(r.errors.is_empty(),"PhysX: {:?}",r.errors);
        assert_eq!(r.corpses[0].bodies.len(),16);
        assert_eq!(r.corpses[0].weight,1.);
        let anchors=r.validation_joint_anchors().unwrap();
        eprintln!("PROBE final joint anchors random{random}: {anchors:?}");
        assert_eq!(anchors.len(),15,"Every original D6 must be connected");
        for (name,error,tolerance,projected) in anchors {
            assert!(error.is_finite(),"Nonfinite joint anchor error: {name}");
            // Fixture numerical allowance only; does not change the original projection setting.
            if projected {assert!(error<=tolerance+0.05,"Disconnected joint {name}: {error}cm > original projection {tolerance}cm + fixture numerical allowance");}
        }
        for (_,id) in &r.corpses[0].bodies {
            let t=r.scene.pose(*id).unwrap();
            assert!(t.position.iter().chain(&t.rotation).all(|v|v.is_finite()));
            assert!(t.position[2]>-30.,"Body penetrated floor: {t:?}");
        }
        let after={let p=s.posed.borrow();s.geo.bone_world(&p["corpse"],hips).loc.z};
        eprintln!("death random{random}: hips {before}→{after} cm");
        assert!(after<before-20.,"Corpse stayed upright: {before}→{after}");
        s.respawn(fi,FVector::new(0.,0.,96.),0.);
        assert!(s.ragdolls.as_ref().unwrap().corpses.is_empty(),"Respawn must release old native actors/joints");
    }
}

#[test]
fn death_clip_selection_matches_blueprint_buckets() {
    let select=mh_sim::ragdoll::death_sequence;
    assert!(select(0.,0,"Head",1,0).ends_with("Death_BackFront1"));
    assert!(select(0.,1,"Head",1,0).ends_with("Death_0"));
    assert!(select(180.,1,"Spine",1,2).ends_with("Death_Frontstomachhit"));
    assert!(select(-180.,1,"Spine",1,3).ends_with("Death_Frontstomachhit"));
    assert!(select(90.,1,"Spine",1,2).ends_with("Death_2"));
    assert!(select(-90.,5,"Head",1,0).ends_with("Death_Frontfall2"));
}

#[test]
fn traces_hit_posed_bodies() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 120.0) else { return eprintln!("SKIP: no character records") };
    assert_eq!(s.geo.shapes.len(), 17, "UMA_Master_PhysicsAsset: 16 bodies, 17 boxes");
    assert!(s.geo.shape_bones.iter().all(|b| b.is_some()), "every body bone is in the skeleton");
    let ls = &s.geo.weapons[LS];
    assert!(ls.trace_start.is_some() && ls.trace_end.is_some(), "Longsword TraceStart / TraceEnd sockets");
    // capsule centre: half height above the floor
    let z = 100.0;
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, z), ..Default::default() });
    // B stands where A's blade (reference pose, static) passes through B's "Spine" body: the blade midpoint minus the
    // Spine box centre's offset from the actor, on the floor
    let (mid, spine_off) = {
        let posed = s.posed.borrow();
        let pa = &posed["A"];
        let wx = s.geo.weapon_world(pa, &s.geo.weapons[LS]).unwrap();
        let g = &s.geo.weapons[LS];
        let (p, q) = (wx.apply(g.trace_start.unwrap()), wx.apply(g.trace_end.unwrap()));
        let mid = FVector::new((p.x + q.x) * 0.5, (p.y + q.y) * 0.5, (p.z + q.z) * 0.5);
        let k = s.geo.shapes.iter().position(|x| x.bone == "Spine").unwrap();
        let c = s.geo.shapes[k].xf.then(&s.geo.bone_world(pa, s.geo.shape_bones[k].unwrap())).loc;
        (mid, FVector::new(c.x - pa.actor.loc.x, c.y - pa.actor.loc.y, c.z - pa.actor.loc.z))
    };
    eprintln!("blade mid {mid:?}, spine offset {spine_off:?}");
    let b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: LS.into(), location: FVector::new(mid.x - spine_off.x, mid.y - spine_off.y, z), ..Default::default() });
    let c = s.add_fighter(&FighterDesc { name: "C".into(), weapon: LS.into(), location: FVector::new(500.0, 0.0, z), ..Default::default() });
    let _ = (b, c);
    let mut hits = Vec::new();
    for n in 0..240 {
        let inp = if n == 1 { vec![(a, SimInput { attack: Some((0, 0.0)), ..Default::default() })] } else { vec![] };
        s.step(&inp);
        let (ev, _) = s.drain();
        hits.extend(ev.into_iter().filter(|x| x["kind"] == "hit"));
    }
    eprintln!("hits: {hits:?}");
    {
        let posed = s.posed.borrow();
        let pa = &posed["A"];
        let wx = s.geo.weapon_world(pa, &s.geo.weapons[LS]);
        eprintln!("A weapon world {:?} tracer {:?}", wx, pa.tracer);
        for (k, sh) in s.geo.shapes.iter().enumerate() {
            let b = s.geo.shape_bones[k].unwrap();
            eprintln!("B {} at {:?} {:?}", sh.bone, sh.xf.then(&s.geo.bone_world(&posed["B"], b)).loc, sh.shape);
        }
        eprintln!("RightWeapon idx {:?}", s.geo.skeleton.find("RightWeapon"));
    }
    assert!(!hits.is_empty(), "A's Release never traced into B (blade through B's Spine, reference pose)");
    assert!(hits.iter().any(|h| h["bone"] == "Spine"), "the blade lies in B's Spine body: {hits:?}");
    assert!(hits.iter().all(|h| h["victim"] == "B"), "only the overlapping fighter is hit: {hits:?}");
    let snap = s.snapshot();
    assert!(snap["f"]["B"]["health"].as_i64().unwrap() < 100);
    assert_eq!(snap["f"]["C"]["health"].as_i64().unwrap(), 100);
}

/// A Longsword right strike swept by its own clip (2H_Sword_RightStrike, the Greatsword RightStrike motion's
/// Animation, decoded by mh-assets with the exe's sampler) through a fighter standing in front: the moving blade's
/// swept segments hit B's bodies. UNCONFIRMED: the clip plays at rate 1 from the attack's StartTime here; the
/// reference's animation layer (godot/game/anim/motion_anim.gd) maps the windup / release normalized times onto the
/// montage instead (not ported yet).
#[test]
fn clip_swing_hits_fighter_in_front() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 120.0) else { return eprintln!("SKIP: no character records") };
    let src = mh_assets::pak_source::PakSource::new(e.vfs.clone());
    let clip = mh_assets::anim::decode(&src, "Mordhau/Content/Mordhau/Animations/RawClips/2H/Sword/2H_Sword_RightStrike").expect("clip");
    eprintln!("clip {} s, {} tracks", clip.sequence_length, clip.tracks.len());
    let mut results = Vec::new();
    for dist in [60.0f32, 80.0, 100.0, 120.0] {
        let Some(mut s2) = sim(&e, 1.0 / 120.0) else { return };
        let a = s2.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
        let _b = s2.add_fighter(&FighterDesc { name: "B".into(), weapon: LS.into(), location: FVector::new(dist, 0.0, 100.0), yaw: 180.0, ..Default::default() });
        let mut hits = Vec::new();
        let mut start: Option<f64> = None;
        for n in 0..200 {
            let inp = if n == 1 { vec![(a, SimInput { attack: Some((0, 0.0)), ..Default::default() })] } else { vec![] };
            // pose for this frame: the clip at (now + dt - StartTime), the attack's time at this step
            if let Some(st) = start {
                let t = (s2.combat.now + s2.dt as f64 - st) as f32;
                let pose = s2.geo.skeleton.sample(&clip, t.min(clip.sequence_length));
                s2.set_pose(a, pose);
            }
            s2.step(&inp);
            if start.is_none() && s2.combat.cur_m(a).map(|m| m.is_attack()).unwrap_or(false) {
                start = Some(s2.combat.cur_m(a).unwrap().start_time);
            }
            let (ev, _) = s2.drain();
            hits.extend(ev.into_iter().filter(|x| x["kind"] == "hit").map(|x| (x["bone"].as_str().unwrap().to_string(), x["t"].as_f64().unwrap())));
        }
        eprintln!("B at {dist} cm: hits {hits:?}");
        results.push(hits);
    }
    let _ = &mut s;
    assert!(results.iter().all(|h| h.len() == 1), "one hit per release at every distance: {results:?}");
    // the blade reaches a farther fighter later in the swing
    let times: Vec<f64> = results.iter().map(|h| h[0].1).collect();
    assert!(times.windows(2).all(|w| w[0] <= w[1]), "hit times {times:?}");
}

/// Two BT_Deathmatch Knight bots (mh-mode) fight on the Sim: movement (ExeMovement), facing and move requests from the
/// bots, attacks swept by their clips (anim.rs stand-in) through the opponent's physics-asset bodies - no scripted
/// contacts. Records: the bots_duel golden header + mode_kismet.json (as mh-mode's closed_loop test).
#[test]
fn bots_fight_with_real_traces() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 60.0) else { return eprintln!("SKIP: no character records") };
    let root = std::env::var("MORDHAU_REPO").map(std::path::PathBuf::from).unwrap_or_else(|_| core_tests().join("../.."));
    let Ok(kt) = std::fs::read_to_string(root.join("godot/data_gen/mode/mode_kismet.json")) else { return eprintln!("SKIP: no mode_kismet.json") };
    let Ok(ht) = std::fs::read_to_string(core_tests().join("golden/mode/bots_duel.jsonl")) else { return eprintln!("SKIP: no bots_duel golden") };
    let mut h: Value = serde_json::from_str(ht.lines().nth(1).unwrap()).unwrap();
    mordhau_core::data::decode_exact(&mut h);
    let k = Arc::new(mh_mode::kismet::Kismet::from_json(&kt).unwrap());
    let profile: mh_mode::ai::Profile = serde_json::from_value(h["bots"][0]["profile"].clone()).unwrap();
    let tree: mh_mode::ai::TreeDef = serde_json::from_value(h["bots"][0]["tree"].clone()).unwrap();
    let walk = h["bodies"][0]["max_walk_speed"].as_f64().unwrap();
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    let b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: LS.into(), location: FVector::new(150.0, 0.0, 100.0), yaw: 180.0, ..Default::default() });
    let mut bots = mh_mode::ai::Bots { k: k.clone(), ..Default::default() };
    for (fi, yaw) in [(a, 0.0), (b, 180.0)] {
        let mut body = mh_mode::ai::BotBody::new(&s.combat.fighters[fi].name);
        body.location = s.movers[fi].location;
        body.yaw = yaw;
        body.weapon_length = s.combat.fighters[fi].tracer.length;
        body.max_walk_speed = walk;
        body.pawn = mh_mode::ai::combat_host::pawn_view(&s.combat, fi);
        body.weapon = mh_mode::ai::combat_host::weapon_view(&s.combat, fi);
        body.inventory = vec![mh_mode::ai::InventoryItem { weapon: body.weapon.clone(), is_fists: false, ranged: false }];
        bots.add_body(body);
        s.bot_fighter.push(fi);
    }
    for bi in 0..2 {
        let now = s.combat.now;
        let mut host = mh_mode::ai::combat_host::CombatHost { world: &mut s.combat, fighter_of: s.bot_fighter.clone() };
        bots.add_bot(bi, profile.clone(), Some(&tree), now, &mut host);
    }
    s.bots = Some(bots);
    let mut poser = mh_sim::anim::ClipPoser::new(mh_assets::pak_source::PakSource::new(e.vfs.clone()));
    let mut hits = Vec::new();
    let mut parries = 0;
    for _ in 0..(90 * 60) {
        let inp = s.bot_inputs();
        poser.pose(&mut s);
        s.step(&inp);
        let (ev, _) = s.drain();
        for x in ev {
            if x["kind"] == "hit" {
                hits.push((x["attacker"].as_str().unwrap().to_string(), x["bone"].as_str().unwrap().to_string(), x["t"].as_f64().unwrap()));
            }
            if x["kind"] == "parry" {
                parries += 1;
            }
        }
        if s.combat.fighters.iter().any(|f| f.dead) {
            break;
        }
    }
    let dead: Vec<&str> = s.combat.fighters.iter().filter(|f| f.dead).map(|f| f.name.as_str()).collect();
    let snap = s.snapshot();
    eprintln!("bots: {} traced hits {:?}, {} parries, dead {:?} at t={:.2}; A at {} B at {}", hits.len(), hits, parries, dead, s.combat.now, snap["f"]["A"]["location"], snap["f"]["B"]["location"]);
    assert!(!hits.is_empty(), "no traced hit in 90 s of bot fighting");
    assert_eq!(dead.len(), 1, "the bot fight ends in a kill");
}

/// The ported animation graph (animgraph.rs) drives the traced pose: a Longsword right strike, posed by its dynamic
/// montage (windup / release positions from the motion, AutoBlend, slots, LayeredBoneBlend_1), hits a fighter in front;
/// snapshot() carries the very local pose the traces used.
#[test]
fn anim_graph_poses_traces_and_snapshot() {
    let Some(e) = env() else { return };
    let mut hits_at = Vec::new();
    for dist in [70.0f32, 100.0] {
        let Some(mut s) = sim(&e, 1.0 / 120.0) else { return eprintln!("SKIP: no character records") };
        s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
        let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
        let _b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: LS.into(), location: FVector::new(dist, 0.0, 100.0), yaw: 180.0, ..Default::default() });
        let mut hits = Vec::new();
        let mut moved = false;
        for n in 0..200 {
            let inp = if n == 1 { vec![(a, SimInput { attack: Some((0, 0.0)), ..Default::default() })] } else { vec![] };
            s.step(&inp);
            let snap = s.snapshot();
            let pose = snap["f"]["A"]["pose"].as_array().unwrap();
            assert_eq!(pose.len(), snap["bones"].as_array().unwrap().len());
            // snapshot pose == traced pose (component bones in `posed`)
            let local = s.local_pose(a);
            let cs = s.geo.skeleton.to_component(&local);
            let traced = s.posed.borrow()["A"].bones.clone();
            for (x, y) in cs.iter().zip(traced.iter()) {
                assert!((x.loc - y.loc).length() < 1e-2, "snapshot pose differs from the traced pose");
            }
            if s.fanim[a].ma.montages.insts.iter().any(|i| i.weight(&s.combat.spec, s.combat.now) > 0.5) {
                moved = true;
            }
            if n % 10 == 0 && std::env::var("ANIM_DEBUG").is_ok() {
                let rw = s.geo.skeleton.find("RightWeapon").unwrap();
                let ma = &s.fanim[a].ma;
                let ws: Vec<(f64, f64)> = ma.montages.insts.iter().map(|i| (i.weight(&s.combat.spec, s.combat.now), i.position(s.combat.now))).collect();
                eprintln!("n {n} seq {} pos {:.3} insts {ws:?} hand {:?} motion {:?}", ma.seq, ma.position, cs[rw].loc, s.combat.cur_m(a).map(|m| m.kind()));
            }
            let (ev, _) = s.drain();
            hits.extend(ev.into_iter().filter(|x| x["kind"] == "hit").map(|x| (x["bone"].as_str().unwrap().to_string(), x["t"].as_f64().unwrap())));
        }
        eprintln!("graph: B at {dist} cm: hits {hits:?} (blend in {}, auto {} offset {})", s.fanim[a].ma.blend_in, s.fanim[a].ma.auto_used, s.fanim[a].ma.offset);
        assert!(moved, "the attack montage never weighed in");
        hits_at.push(hits);
    }
    assert!(hits_at.iter().all(|h| !h.is_empty()), "the montage-posed swing hits the fighter in front: {hits_at:?}");
}

/// A parrying defender facing the swing: the traced blade crosses B's enabled BlockCollider (BlockedAttacks memory)
/// before its body, so the body hit is parried (exe CheckParry geometry, exe.rs difference 13)
#[test]
fn traced_parry_blocks_the_swing() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 120.0) else { return eprintln!("SKIP: no character records") };
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
    eprintln!("block collider rel {:?} extent {:?}", s.geo.block_collider_rel, s.geo.block_collider_extent);
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    let b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: LS.into(), location: FVector::new(70.0, 0.0, 100.0), yaw: 180.0, ..Default::default() });
    let mut parried = false;
    let mut hit = false;
    let mut asked = false;
    for n in 0..200 {
        let mut inp = Vec::new();
        if n == 1 {
            inp.push((a, SimInput { attack: Some((0, 0.0)), ..Default::default() }));
        }
        if !asked {
            if let Some(at) = s.combat.cur_m(a).and_then(|m| m.attack()) {
                // the graph-posed blade reaches B's head ~0.3 s into the release: parry 0.15 s into it
                if s.combat.now + s.dt as f64 >= at.windup_end + 0.15 {
                    inp.push((b, SimInput { parry: Some(0), ..Default::default() }));
                    asked = true;
                }
            }
        }
        s.step(&inp);
        if std::env::var("ANIM_DEBUG").is_ok() {
            let p = s.posed.borrow();
            eprintln!("t {:.3} B {:?} bc_on {} tracer {:?}", s.combat.now, s.combat.cur_m(b).map(|m| m.kind()), s.combat.fighters[b].block_collider_enabled, p["A"].tracer);
            if n % 10 == 0 {
                eprintln!("  B geom {:?}", s.combat.fighters[b].geom);
                if let Some(pm) = s.combat.fighters[b].last_parry_motion {
                    eprintln!("  B blocked_attacks {:?}", s.combat.m(b, pm).parry().map(|p| p.blocked_attacks.clone()));
                }
            }
        }
        let (ev, _) = s.drain();
        for x in &ev {
            eprintln!("event {x:?}");
        }
        parried |= ev.iter().any(|x| x["kind"] == "parry");
        hit |= ev.iter().any(|x| x["kind"] == "hit");
    }
    assert!(parried && !hit, "parried {parried} hit {hit}");
}

/// The game mode inside the Sim (mode.rs): an FFA (ffa_kills golden's ModeData) logs two controllers in, spawns their
/// pawns at PlayerStarts, scores a traced kill (on_damage / on_killed from the drain events) and respawns the victim
#[test]
fn ffa_mode_spawns_scores_and_respawns() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 60.0) else { return eprintln!("SKIP: no character records") };
    let root = std::env::var("MORDHAU_REPO").map(std::path::PathBuf::from).unwrap_or_else(|_| core_tests().join("../.."));
    let Ok(t) = std::fs::read_to_string(core_tests().join("golden/mode/ffa_kills.jsonl")) else { return eprintln!("SKIP: no ffa golden") };
    let mut h: Value = serde_json::from_str(t.lines().nth(1).unwrap()).unwrap();
    mordhau_core::data::decode_exact(&mut h);
    let data: mh_mode::data::ModeData = serde_json::from_value(h["data"].clone()).unwrap();
    let respawn = data.scoring.player_respawn_time;
    let k = mh_mode::kismet::Kismet::from_json(&std::fs::read_to_string(root.join("godot/data_gen/mode/mode_kismet.json")).unwrap()).unwrap();
    s.set_mode(mh_mode::game_mode::GameMode::new(data, std::sync::Arc::new(k), mordhau_core::ue::CrtRand::new(1)));
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
    s.join("A", false, LS, "");
    s.join("B", false, LS, "");
    let mut spawns = Vec::new();
    let mut kill_t = None;
    let mut attacked = false;
    for _ in 0..(60 * 20) {
        let ia = s.combat.fighter_index("A");
        let ib = s.combat.fighter_index("B");
        let mut inp = Vec::new();
        if let (Some(a), Some(b)) = (ia, ib) {
            // once both stand: B in front of A, facing it; A swings
            if !attacked && s.combat.now > 2.0 && !s.combat.fighters[b].dead {
                let la = s.movers[a].location;
                s.movers[b].location = FVector::new(la.x + 70.0, la.y, la.z);
                s.yaw[a] = 0.0;
                s.yaw[b] = 180.0;
                inp.push((a, SimInput { attack: Some((0, 0.0)), ..Default::default() }));
                attacked = true;
            }
        }
        s.step(&inp);
        for ev in s.drain_mode() {
            if ev.kind == "spawn_pawn" {
                spawns.push((ev.get("who").unwrap().as_s().to_string(), s.combat.now));
            }
        }
        if kill_t.is_none() && s.combat.fighters.iter().any(|f| f.dead) {
            kill_t = Some(s.combat.now);
        }
        if kill_t.is_some() && spawns.iter().filter(|x| x.0 == "B").count() >= 2 {
            break;
        }
    }
    let m = s.mode.as_ref().unwrap();
    let (ca, cb) = (m.gm.by_name("A").unwrap(), m.gm.by_name("B").unwrap());
    eprintln!("spawns {spawns:?} kill at {kill_t:?}; A kills {} score {}, B deaths {}", m.gm.ctrls[ca].kills, m.gm.ctrls[ca].score, m.gm.ctrls[cb].deaths);
    assert!(spawns.iter().any(|x| x.0 == "A") && spawns.iter().any(|x| x.0 == "B"), "both pawns spawned at PlayerStarts");
    let kt = kill_t.expect("the traced swing kills B");
    assert_eq!(m.gm.ctrls[ca].kills, 1);
    assert_eq!(m.gm.ctrls[cb].deaths, 1);
    assert!(m.gm.ctrls[ca].score > 0.0);
    let re = spawns.iter().filter(|x| x.0 == "B").nth(1).expect("B respawns").1;
    assert!(re - kt >= respawn - 0.05, "respawn after PlayerRespawnTime {respawn}: died {kt}, respawned {re}");
    assert!(!s.combat.fighters[s.combat.fighter_index("B").unwrap()].dead, "the new pawn is alive");
}

/// Alternate mode traces from the weapon's SecondTraceStart / SecondTraceEnd sockets (GetTrace_Implementation
/// rva=0x1629520)
#[test]
fn alternate_mode_uses_second_sockets() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 120.0) else { return eprintln!("SKIP: no character records") };
    let g = s.geo.weapons[LS].clone();
    eprintln!("Longsword sockets {:?} {:?} / second {:?} {:?}", g.trace_start, g.trace_end, g.second_trace_start, g.second_trace_end);
    assert!(g.second_trace_start.is_some() && g.second_trace_end.is_some(), "BP_Longsword's mesh has SecondTraceStart / SecondTraceEnd");
    assert_ne!(g.second_trace_start, g.trace_start);
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    let host = s.combat.trace_host.clone().unwrap();
    host.prepare(&s.combat, a);
    let primary = s.posed.borrow()["A"].tracer.cur_start;
    s.combat.fighters[a].alternate_mode = true;
    host.prepare(&s.combat, a);
    let alt = s.posed.borrow()["A"].tracer.cur_start;
    let wx = s.geo.weapon_world(&s.posed.borrow()["A"], &g).unwrap();
    assert_eq!(primary, wx.apply(g.trace_start.unwrap()));
    assert_eq!(alt, wx.apply(g.second_trace_start.unwrap()));
}

/// Weapon-on-weapon clash through the traced ClashCollider capsules (RepositionClashCollider rva=0x163b150): two
/// fighters swing mirrored strikes at each other at the same time; a blade sweeps through the other's enabled clash
/// capsule and CheckClash rva=0x16179a0 decides. Prints the outcome per distance; asserts the capsules are traced
/// (a Clash-component hit reaches the combat) at one distance at least.
#[test]
fn clash_capsules_are_traced() {
    let Some(e) = env() else { return };
    let mut any_clash = false;
    for dist in [90.0f32, 110.0, 130.0, 150.0] {
        let Some(mut s) = sim(&e, 1.0 / 120.0) else { return eprintln!("SKIP: no character records") };
        s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
        let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
        let b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: LS.into(), location: FVector::new(dist, 0.0, 100.0), yaw: 180.0, ..Default::default() });
        let mut kinds = Vec::new();
        for n in 0..160 {
            let inp = if n == 1 { vec![(a, SimInput { attack: Some((0, 0.0)), ..Default::default() }), (b, SimInput { attack: Some((0, 0.0)), ..Default::default() })] } else { vec![] };
            s.step(&inp);
            if n == 3 {
                let p = s.posed.borrow();
                eprintln!("  clash capsule A {:?}", p["A"].clash);
            }
            let (ev, _) = s.drain();
            for x in ev {
                let k = x["kind"].as_str().unwrap_or("").to_string();
                if k == "clash" || k == "hit" || k == "parry" || k == "chamber" {
                    kinds.push((k, x["t"].as_f64().unwrap_or(0.0)));
                }
            }
        }
        eprintln!("clash test at {dist} cm: {kinds:?}");
        any_clash |= kinds.iter().any(|k| k.0 == "clash");
    }
    assert!(any_clash, "no clash at any distance");
}

/// The 1P camera (UpdateFPCamera: "Position" bone rotation + "Spine1" location, geometry::camera_1p) sits at eye
/// height and looks along the actor's yaw, pitched by LookUpValue + FirstPersonLookUpOffset sign-consistently
#[test]
fn camera_1p_follows_actor_and_look() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 120.0) else { return eprintln!("SKIP: no character records") };
    for yaw in [0.0f32, 90.0, -135.0] {
        let name = format!("F{yaw}");
        let fi = s.add_fighter(&FighterDesc { name: name.clone(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), yaw, ..Default::default() });
        s.refresh_geom(fi);
        let g = s.combat.fighters[fi].geom.unwrap();
        eprintln!("yaw {yaw}: camera {:?} rot {:?}", g.camera_loc, g.camera_rot);
        let dy = ((g.camera_rot.1 - yaw + 540.0) % 360.0) - 180.0;
        assert!(dy.abs() < 1.0, "camera yaw {} vs actor {yaw}", g.camera_rot.1);
        assert!(g.camera_loc.z > 140.0 && g.camera_loc.z < 200.0, "eye height {}", g.camera_loc.z);
        s.combat.fighters[fi].look_up_value = 30.0;
        s.refresh_geom(fi);
        let up = s.combat.fighters[fi].geom.unwrap().camera_rot.0;
        s.combat.fighters[fi].look_up_value = -30.0;
        s.refresh_geom(fi);
        let down = s.combat.fighters[fi].geom.unwrap().camera_rot.0;
        eprintln!("  pitch at look +30: {up}, -30: {down}");
        assert!((up - down).abs() > 50.0, "look changes the pitch");
    }
}

/// The look-up spine bend after AttackAngling (procedural.rs, UMordhauAnimInstance helpers): looking up moves the
/// traced RightWeapon bone; looking 30 deg up rolls Spine1 by -30 in component space relative to look 0
#[test]
fn look_up_bends_the_traced_pose() {
    let Some(e) = env() else { return };
    let mut at = Vec::new();
    for look in [0.0f64, 30.0, -30.0] {
        let Some(mut s) = sim(&e, 1.0 / 120.0) else { return eprintln!("SKIP: no character records") };
        s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
        let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
        for _ in 0..30 {
            s.step(&[(a, SimInput { look_up: Some(look), ..Default::default() })]);
        }
        let rw = s.geo.skeleton.find("RightWeapon").unwrap();
        let s1 = s.geo.skeleton.find("Spine1").unwrap();
        let cs = s.posed.borrow()["A"].bones.clone();
        at.push((cs[rw].loc, cs[s1].rot));
    }
    eprintln!("look-up weapon bone: {:?}", at.iter().map(|x| x.0).collect::<Vec<_>>());
    assert!((at[1].0 - at[0].0).length() > 5.0, "look up moved the weapon bone");
    assert!(at[1].0.z > at[0].0.z && at[2].0.z < at[0].0.z, "looking up raises the weapon, down lowers it");
}

/// Turn caps (mordhau-core turncap.rs): during an attack the Longsword's TurnCaps (250 deg/s from the spec matrix)
/// rate-limit SimInput.yaw; without an attack a yaw input applies at once
#[test]
fn attack_turn_caps_limit_yaw() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 120.0) else { return eprintln!("SKIP: no character records") };
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    s.step(&[(a, SimInput { yaw: Some(30.0), ..Default::default() })]);
    assert_eq!(s.yaw[a], 30.0, "no attack: uncapped");
    s.step(&[(a, SimInput { attack: Some((0, 0.0)), ..Default::default() })]);
    let caps = s.combat.fighters[a].turn_caps;
    eprintln!("caps after attack start: {caps:?}");
    assert!(caps.turn_rate_cap > 0.0, "the attack set a turn cap");
    let y0 = s.yaw[a];
    for _ in 0..12 {
        s.step(&[(a, SimInput { yaw: Some(y0 + 90.0), ..Default::default() })]);
    }
    let turned = s.yaw[a] - y0;
    eprintln!("turned {turned} deg in 0.1 s (cap {})", caps.turn_rate_cap);
    assert!(turned > 0.0 && turned <= (caps.turn_rate_cap * 0.1 + caps.turn_rate_cap * 0.0333 + 1e-3) as f32, "{turned}");
}

/// parry_angle proof B2 (state/proofs/parry_angle.md): a parry caps turning at the weapon's ParryTurnCap
/// (BP_MordhauWeapon (375, 262.5) deg/s; UParryMotion::OnBegin decomp 919-934), and leaving it clears the cap (OnLeave
/// decomp 2119-2120)
#[test]
fn parry_turn_cap_limits_yaw() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 120.0) else { return eprintln!("SKIP: no character records") };
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    s.step(&[]);
    s.step(&[(a, SimInput { parry: Some(0), ..Default::default() })]);
    let caps = s.combat.fighters[a].turn_caps;
    assert_eq!((caps.turn_rate_cap, caps.look_up_rate_cap), (375.0, 262.5), "{caps:?}");
    let y0 = s.yaw[a];
    for _ in 0..12 {
        s.step(&[(a, SimInput { yaw: Some(y0 + 90.0), ..Default::default() })]);
    }
    let turned = s.yaw[a] - y0;
    assert!(turned > 0.0 && turned <= (375.0 * 0.1 + 375.0 * 0.0333 + 1e-3) as f32, "turned {turned}");
    // the parry ends (released, recovered): the cap goes
    for _ in 0..240 {
        s.step(&[(a, SimInput { release_block: true, ..Default::default() })]);
    }
    assert!(!s.combat.cur_m(a).map(|m| m.is_parry()).unwrap_or(false), "parry over");
    assert_eq!(s.combat.fighters[a].turn_caps.turn_rate_cap, -1.0);
}

/// The upper blend space and the Additive machine load from the paks: the Longsword's UpperBlendSpace grid is
/// complete, and a parry drives the machine into its Parry state with the parry's additive clip
#[test]
fn upper_blend_space_and_parry_additive() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 120.0) else { return eprintln!("SKIP: no character records") };
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    let bs = s.fanim[a].upper_bs.clone().expect("Longsword UpperBlendSpace");
    eprintln!("upper bs {} samples {} grid {} n {:?}", bs.path, bs.samples.len(), bs.grid.len(), bs.grid_n);
    assert!(!bs.samples.is_empty());
    assert_eq!(bs.grid.len() as i64, (bs.grid_n.0 + 1) * (bs.grid_n.1 + 1));
    s.step(&[]);
    s.step(&[(a, SimInput { parry: Some(0), ..Default::default() })]);
    let mut saw_parry = false;
    for _ in 0..30 {
        s.step(&[]);
        let ma = &s.fanim[a].ma;
        if ma.machine.current == 3 && ma.additive_alpha > 0.5 {
            saw_parry = true;
        }
    }
    eprintln!("parry inputs {:?}", s.fanim[a].ma.machine.inputs);
    assert!(saw_parry, "the Additive machine entered Parry");
    let an = s.anim.clone().unwrap();
    let clip = an.single_clip(&s.fanim[a].ma.machine.inputs.parry_additive);
    eprintln!("parry additive clip {clip}");
    assert!(!clip.is_empty() && an.clip(&clip).is_some(), "the parry blend space has one decodable clip");
}

const ARROW: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Ranged/ArrowVersions/BP_LongbowArrow";
const FISTS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/BP_FistsWeapon";

/// Ranged (mh-sim ranged.rs + mordhau-core ranged.rs): an arrow fired at a standing fighter flies, hits a posed body
/// and deals ComputeRangedDamage through TakeDamage (health drops, a ranged hit event)
#[test]
fn arrow_hits_and_damages() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 120.0) else { return eprintln!("SKIP: no character records") };
    let rd = mh_pak::Reader::new(e.vfs.clone());
    let (cfg, dmg) = mh_sim::ranged::projectile_class(&rd, ARROW).expect("arrow class");
    eprintln!("arrow damage {:?} speed {}", dmg, cfg.initial_speed);
    assert!(!dmg.damage.is_empty(), "the arrow has damage values");
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    let b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: LS.into(), location: FVector::new(1000.0, 0.0, 100.0), yaw: 180.0, ..Default::default() });
    s.step(&[]);
    let h0 = s.combat.fighters[b].health;
    s.fire_projectile(Some(a), cfg, dmg, FVector::new(0.0, 0.0, 150.0), 0.0, 0.0);
    let mut ranged_hit = None;
    for _ in 0..120 {
        s.step(&[]);
        let (ev, _) = s.drain();
        if let Some(h) = ev.into_iter().find(|x| x["kind"] == "hit" && x["ranged"] == true) {
            ranged_hit = Some(h);
        }
    }
    eprintln!("ranged hit {ranged_hit:?} outcomes {:?}", s.projectiles[0].outcomes);
    assert!(ranged_hit.is_some(), "the arrow hit B");
    assert!(s.combat.fighters[b].health < h0, "B lost health");
}

/// Horses (mh-sim horse.rs + mordhau-core horse.rs): a rider mounts (EnterVehicle motion, the Longsword holstered or
/// kept per bCanEquipOnHorse), rides forward, and the galloping horse tramples a fighter standing in its path
/// (DoKnockback: damage, knockback); then dismounts
#[test]
fn horse_mount_ride_trample() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 60.0) else { return eprintln!("SKIP: no character records") };
    let rd = mh_pak::Reader::new(e.vfs.clone());
    let h = match mh_sim::horse::load_horse(&rd, &s.records, FVector::new(0.0, 0.0, 125.0), 0.0) {
        Ok(h) => h,
        Err(err) => return eprintln!("SKIP: horse data ({err})"),
    };
    eprintln!("horse combat {:?} bump {:?}", h.combat, h.bump);
    let hi = s.add_horse(h);
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 80.0, 100.0), ..Default::default() });
    let b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: LS.into(), location: FVector::new(1500.0, 0.0, 100.0), yaw: 180.0, ..Default::default() });
    for _ in 0..10 {
        s.step(&[]);
    }
    let flags = mh_sim::horse::equip_on_horse_flags(&rd, LS);
    assert!(s.mount(a, hi, flags, FISTS), "A mounts the horse");
    assert!(s.combat.entering_or_leaving_vehicle(a), "the EnterVehicle motion runs");
    let h0 = s.combat.fighters[b].health;
    let mut trampled = false;
    for _ in 0..360 {
        s.step(&[(a, SimInput { fwd: 1.0, ..Default::default() })]);
        let (ev, _) = s.drain();
        trampled |= ev.iter().any(|x| x["kind"] == "hit" && x["horse"] == true);
    }
    eprintln!("horse at {:?} speed {} B health {} -> {}", s.horses[hi].m.location, s.horses[hi].m.velocity.length(), h0, s.combat.fighters[b].health);
    assert!(s.horses[hi].m.location.x > 300.0, "the rider drove the horse forward");
    assert!((s.movers[a].location - s.horses[hi].m.location).length() < 300.0, "the rider rides along");
    assert!(trampled, "the horse trampled B");
    assert!(s.dismount(a, None), "A dismounts");
    assert!(s.riding(a).is_none());
}

const LONGBOW: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Ranged/BP_Longbow";

/// A full bow cycle (mordhau-core rangedmotion.rs + mh-sim ranged.rs): fire held while unloaded -> Reload (1.1 s,
/// loaded at 0.6), still held -> Draw (1.0 s), fire released after the draw time -> Release fires the longbow arrow,
/// which hits the fighter 10 m ahead
#[test]
fn bow_cycle_reload_draw_release_hits() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 120.0) else { return eprintln!("SKIP: no character records") };
    let rd = mh_pak::Reader::new(e.vfs.clone());
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    let b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: LS.into(), location: FVector::new(1000.0, 0.0, 100.0), yaw: 180.0, ..Default::default() });
    assert!(s.equip_ranged(a, &rd, LONGBOW), "the longbow can fire");
    s.step(&[(a, SimInput { fire: Some(true), look_up: Some(-9.0), ..Default::default() })]);
    let mut kinds: Vec<(f64, String)> = Vec::new();
    let mut hit = false;
    let mut sway = None;
    for n in 0..600 {
        // hold fire through the reload and the draw, release 1.5 s into the draw
        let draw_since = kinds.iter().find(|k| k.1 == "RangedDraw").map(|k| k.0);
        let release = draw_since.map(|t| s.combat.now - t > 1.5).unwrap_or(false);
        let inp = if release { vec![(a, SimInput { fire: Some(false), ..Default::default() })] } else { vec![] };
        s.step(&inp);
        let k = s.combat.cur_m(a).map(|m| m.kind().to_string()).unwrap_or_default();
        if let Some(x) = s.combat.fighters[a].motion.and_then(|id| s.combat.ranged_m(a, id)) {
            if k == "RangedDraw" {
                sway = Some(x.aim_offset);
            }
        }
        if kinds.last().map(|x| x.1 != k).unwrap_or(true) {
            kinds.push((s.combat.now, k));
        }
        let (ev, _) = s.drain();
        hit |= ev.iter().any(|x| x["kind"] == "hit" && x["ranged"] == true);
        if hit && n > 10 {
            break;
        }
    }
    eprintln!("bow motions {kinds:?} B health {} last draw sway {sway:?}", s.combat.fighters[b].health);
    // VC_LongbowSway (URangedDrawMotion::OnTick_Implementation rva=0x16694b0): the pitch sway is still non-zero 1.3 s into the loop
    let sw = sway.expect("a draw tick");
    assert!(sw.0 != 0.0 && sw.0.abs() < 25.0, "longbow pitch sway {sw:?}");
    eprintln!("fire aim {:?} geom cam {:?}", s.combat.fighters[a].last_fire_aim, s.combat.fighters[a].geom.as_ref().map(|g| (g.camera_loc, g.camera_rot)));
    for p in &s.projectiles {
        eprintln!("projectile at {:?} vel {:?} terminated {} outcomes {:?}", p.p.location, p.p.velocity, p.p.terminated, p.outcomes);
    }
    let order: Vec<&str> = kinds.iter().map(|k| k.1.as_str()).collect();
    for want in ["Reload", "RangedDraw", "RangedRelease"] {
        assert!(order.contains(&want), "{want} in {order:?}");
    }
    assert!(hit, "the arrow hit B");
}

/// Horses as melee trace targets (mh-sim horse.rs horse_traces) and the mounted state: a fighter strikes a riderless
/// horse standing in front of it -> a horse_hit, the horse loses health; a mounted rider can still attack
/// (bCanAttackOnHorseback) and carries its horse's Mount
#[test]
fn horse_is_a_melee_target_and_rider_mounts() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 120.0) else { return eprintln!("SKIP: no character records") };
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
    let rd = mh_pak::Reader::new(e.vfs.clone());
    let h = match mh_sim::horse::load_horse(&rd, &s.records, FVector::new(110.0, 0.0, 125.0), 90.0) {
        Ok(h) => h,
        Err(err) => return eprintln!("SKIP: horse data ({err})"),
    };
    let hi = s.add_horse(h);
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    s.step(&[]);
    let h0 = s.horses[hi].health;
    let mut hit = false;
    for n in 0..240 {
        let inp = if n == 1 { vec![(a, SimInput { attack: Some((0, 0.0)), ..Default::default() })] } else { vec![] };
        s.step(&inp);
        let (ev, _) = s.drain();
        hit |= ev.iter().any(|x| x["kind"] == "horse_hit");
    }
    eprintln!("horse health {h0} -> {}", s.horses[hi].health);
    assert!(hit && s.horses[hi].health < h0, "the swing hit the horse");
    // mount: the Mount is fed, the rider can still attack
    s.horses[hi].m.location = FVector::new(0.0, 60.0, 125.0);
    let flags = mh_sim::horse::equip_on_horse_flags(&rd, LS);
    if s.mount(a, hi, flags, FISTS) {
        for _ in 0..60 {
            s.step(&[]);
        }
        assert!(s.combat.fighters[a].mount.is_some(), "the rider's Mount is set");
        assert!(s.combat.can_perform_attack(a, 0) == s.combat.fighters[a].weapon.as_ref().unwrap().b_can_attack_on_horseback);
    }
}

/// The presentation events (mh-sim events.rs): walking emits foot_landed notifies, an attack one release with its trace,
/// crouching crouch_start / crouch_end, and motions their "motion" begins
#[test]
fn sound_events_are_emitted() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 60.0) else { return eprintln!("SKIP: no character records") };
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    let mut kinds: Vec<String> = Vec::new();
    let mut release = None;
    for n in 0..240 {
        let inp = match n {
            0..=119 => SimInput { fwd: 1.0, ..Default::default() },
            130 => SimInput { attack: Some((0, 0.0)), ..Default::default() },
            200..=219 => SimInput { crouch: true, ..Default::default() },
            _ => SimInput::default(),
        };
        s.step(&[(a, inp)]);
        let (ev, _) = s.drain();
        for x in ev {
            let k = x["kind"].as_str().unwrap_or("").to_string();
            if k == "release" {
                release = Some(x.clone());
            }
            kinds.push(k);
        }
    }
    let count = |k: &str| kinds.iter().filter(|x| *x == k).count();
    eprintln!("feet {} release {:?} crouch {}/{} motions {}", count("foot_landed"), release, count("crouch_start"), count("crouch_end"), count("motion"));
    assert!(count("foot_landed") >= 2, "footsteps while walking");
    assert_eq!(count("release"), 1, "one release per attack");
    assert!(count("motion") >= 2, "motion begins");
    assert!(count("crouch_start") >= 1 && count("crouch_end") >= 1, "crouch edges");
    assert_eq!(count("attack_yell"), 1, "one yell per attack");
    assert_eq!(release.as_ref().and_then(|r| r["yell_emitted"].as_bool()), Some(true), "the yell precedes the release");
}

/// the decoded WeaponSlideAmount / SlideCompensationWeight curves of 2H_Polearm_RightStrike (the pak's
/// CompressedRichCurve bytes) evaluated like FRichCurve::Eval
#[test]
fn weapon_slide_curves_decode() {
    let Some(e) = env() else { return };
    let a = mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets");
    let Some(s) = sim(&e, 1.0 / 60.0) else { return eprintln!("SKIP: no character records") };
    let spec = s.combat.spec.clone();
    let clip = "Mordhau/Content/Mordhau/Animations/RawClips/2H/Polearm/2H_Polearm_RightStrike";
    let amt = |t| a.curve_value(&spec, clip, "WeaponSlideAmount", t).unwrap();
    let w = |t| a.curve_value(&spec, clip, "SlideCompensationWeight", t).unwrap();
    assert!((amt(1.0) - 30.0).abs() < 1e-3 && amt(0.5).abs() < 1e-6 && amt(1.5) > 0.0 && amt(1.5) < 30.0);
    assert!((w(0.55) - 0.5).abs() < 1e-3 && (w(0.8) - 1.0).abs() < 1e-6);
}

/// Sim::step_dt (the client's per-frame tick): at 240 Hz an attack pressed at the same world time reaches its release
/// within one 240 Hz frame of the 60 Hz run's (the motion times are world-time based, UAttackMotion::OnTick)
#[test]
fn variable_dt_step_matches_fixed_timing() {
    let Some(e) = env() else { return };
    let release_at = |hz: f32| -> Option<f64> {
        let mut s = sim(&e, 1.0 / 60.0)?;
        let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
        let mut pressed = false;
        for _ in 0..(hz as usize * 3) {
            let inp = if !pressed && s.combat.now >= 0.5 - 1e-6 {
                pressed = true;
                vec![(a, SimInput { attack: Some((0, 0.0)), ..Default::default() })]
            } else {
                vec![]
            };
            s.step_dt(&inp, 1.0 / hz);
            if s.combat.cur_m(a).and_then(|m| m.attack()).map(|x| x.stage == 1).unwrap_or(false) {
                return Some(s.combat.now);
            }
        }
        None
    };
    let (r60, r240) = (release_at(60.0), release_at(240.0));
    eprintln!("release 60 Hz {r60:?} 240 Hz {r240:?}");
    let (r60, r240) = (r60.expect("60 Hz release"), r240.expect("240 Hz release"));
    assert!((r60 - r240).abs() <= 1.0 / 60.0 + 1e-4, "{r60} vs {r240}");
}

/// First person (UAttackMotion::OnBegin_Implementation decomp 1623-1660 / OnTick_Implementation 2837-2845): the
/// local view target's attack plays the FirstPerson sequence, then switches to the queued ThirdPerson one once the
/// montage position reaches AnimationTimeFor3PTransition (0.6) in the release
#[test]
fn first_person_attack_switches_to_3p_in_release() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 120.0) else { return eprintln!("SKIP: no character records") };
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    s.set_first_person(a, true);
    s.set_view_target(a, true, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
    s.step(&[(a, SimInput { attack: Some((0, 0.0)), ..Default::default() })]);
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..200 {
        s.step(&[]);
        let live: Vec<String> = s.fanim[a].ma.montages.insts.iter().filter(|i| !i.stopped).map(|i| i.asset.path.rsplit('/').next().unwrap_or("").to_string()).collect();
        for p in live {
            if seen.last() != Some(&p) {
                seen.push(p);
            }
        }
    }
    eprintln!("1P attack montages {seen:?}");
    let i1 = seen.iter().position(|p| p.to_lowercase().contains("1p")).expect("the FirstPerson sequence plays");
    assert!(seen[i1 + 1..].iter().any(|p| !p.to_lowercase().contains("1p")), "then the ThirdPerson one: {seen:?}");
}

#[test]
fn view_target_survives_animation_enable_and_same_id_respawn() {
    let Some(e) = env() else { return };
    let mut s = sim(&e, 1.0 / 120.0).expect("original character records");
    s.dedicated_server = false;
    let fi = s.add_fighter(&FighterDesc { name: "camera".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    // Host ownership can arrive before animation assets are enabled.
    s.set_first_person(fi, true);
    s.set_view_target(fi, true, false);
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
    assert!(s.fanim[fi].first_person && s.fanim[fi].is_view_target);
    assert!(s.fanim[fi].ma.first_person && s.fanim[fi].ma.is_view_target);
    for (fp, target, debug_override) in [(false, true, false), (true, true, false), (true, true, true), (false, false, false)] {
        s.set_first_person(fi, fp);
        s.set_view_target(fi, target, debug_override);
        s.respawn(fi, FVector::new(0.0, 0.0, 100.0), 0.0);
        let f = &s.fanim[fi];
        assert_eq!((f.first_person, f.is_view_target, f.view_target_debug_override), (fp, target, debug_override));
        assert_eq!((f.ma.first_person, f.ma.is_view_target, f.ma.view_target_debug_override), (fp, target, debug_override),
            "the first refreshed pose after respawn must already have the current camera state");
        assert_eq!(s.combat.fighters.len(), 1);
        assert_eq!(s.combat.fighters[fi].name, "camera");
    }
    s.set_first_person(fi, true);
    s.set_view_target(fi, true, true);
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("reload anim assets"));
    let a = &s.fanim[fi].ma;
    assert!(a.first_person && a.is_view_target && a.view_target_debug_override);
}

/// The airborne states (air.rs, fidelity-audit r6): a jump drives the UpperBody machine into Jump and the LowerBody
/// machine into Jump with the Longsword's JumpAnimation, and landing ends in Land / Ground; a standalone fighter's
/// pose differs from the dedicated-server pose (UpperAdditive at AnimLOD1, no hips override)
#[test]
fn jump_enters_airborne_states_and_lands() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 120.0) else { return eprintln!("SKIP: no character records") };
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    let an = s.fanim[a].air.anims.clone();
    eprintln!("air anims {an:?}");
    assert!(an.jump.contains("Jump") && an.land.contains("Land"), "{an:?}");
    // settle on the floor past the landing's JumpCooldown
    for _ in 0..360 {
        s.step(&[]);
    }
    let (mut ub_jump, mut lb_jump, mut airborne, mut landed) = (false, false, false, false);
    s.step(&[(a, SimInput { jump: true, ..Default::default() })]);
    for _ in 0..240 {
        s.step(&[]);
        let air = &s.fanim[a].air;
        ub_jump |= air.ub.current == mh_sim::air::UB_JUMP;
        lb_jump |= air.lb.current == mh_sim::air::LB_JUMP;
        airborne |= air.airborne;
        landed |= airborne && !air.airborne && matches!(air.ub.current, mh_sim::air::UB_LAND | mh_sim::air::UB_IDLE);
    }
    assert!(ub_jump && lb_jump && airborne && landed, "{ub_jump} {lb_jump} {airborne} {landed}");
    // the standalone pose
    let server = s.posed.borrow()["A"].bones.clone();
    s.set_server_pose(a, false);
    s.step(&[]);
    let client = s.posed.borrow()["A"].bones.clone();
    let hips = s.geo.skeleton.find("Hips").unwrap();
    eprintln!("hips server {:?} standalone {:?}", server[hips].loc, client[hips].loc);
    assert!((server[hips].loc - client[hips].loc).length() > 0.1, "the standalone pose drops the hips override");
}

/// fidelity-audit r7 diagnostic: the 1P Greatsword idle's hands / RightWeapon in camera space (UE camera axes: X forward,
/// Y right, Z up, cm), standalone vs server pose; run with --ignored --nocapture
#[test]
#[ignore]
fn diag_1p_weapon_in_camera_space() {
    let Some(e) = env() else { return };
    const GS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Greatsword";
    for server in [true, false] {
        let rec = std::fs::read_to_string(core_tests().join("golden/character/records.json")).unwrap();
        let records = CharRecords(&rec).load().unwrap();
        let ld = mh_sim::load::load(&e.matrix, e.vfs.clone(), &[GS]).expect("load");
        let mut floor = BoxWorld::new();
        floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
        let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), records, Box::new(floor), 1.0 / 120.0);
        s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
        let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: GS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
        s.set_server_pose(a, server);
        s.set_first_person(a, true);
        s.set_view_target(a, true, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
        for _ in 0..240 {
            s.step(&[]);
        }
        let sk = s.geo.skeleton.clone();
        let cs = s.posed.borrow()["A"].bones.clone();
        let (pos, sp) = (sk.find("Position").unwrap(), sk.find("Spine1").unwrap());
        let (cam, (p, y)) = mordhau_core::combat::geometry::camera_1p(cs[pos].rot, cs[sp].loc, 1.0, 0.0);
        let q = mordhau_core::ue::FQuat::from_rotator(p, y, 0.0);
        let inv = q.inverse();
        eprintln!("server={server} cam {cam:?} rot ({p:.2}, {y:.2}) upper_bs {:?} add {}", s.fanim[a].upper_bs.as_ref().map(|b| b.path.clone()), s.fanim[a].upper_additive);
        for b in ["RightHand", "LeftHand", "RightWeapon", "head", "Spine1"] {
            if let Some(i) = sk.find(b) {
                let v = inv.rotate(cs[i].loc - cam);
                eprintln!("  {b:12} fwd {:7.2} right {:7.2} up {:7.2}", v.x, v.y, v.z);
            }
        }
        // the raw 1P blend space sample at (Direction, Velocity) = (0, 0), camera from the same pose
        let an = s.anim.clone().unwrap();
        let bs = s.fanim[a].upper_bs.clone().unwrap();
        let raw = sk.to_component(&an.blend_space_pose(&sk, &bs, 0.0, 0.0, 1.0));
        let (cam2, (p2, y2)) = mordhau_core::combat::geometry::camera_1p(raw[pos].rot, raw[sp].loc, 1.0, 0.0);
        let inv2 = mordhau_core::ue::FQuat::from_rotator(p2, y2, 0.0).inverse();
        eprintln!("  raw blend space: cam {cam2:?} rot ({p2:.2}, {y2:.2})");
        for b in ["RightHand", "LeftHand", "RightWeapon", "head"] {
            if let Some(i) = sk.find(b) {
                let v = inv2.rotate(raw[i].loc - cam2);
                eprintln!("  raw {b:12} fwd {:7.2} right {:7.2} up {:7.2}", v.x, v.y, v.z);
            }
        }
    }
}

/// The shield's own BlockCollider in the attack trace (BP_MordhauShield "BlockColliderBP", UAttackMotion::
/// ProcessHitForBlocking rva=0x1638380 shield branch): the box loads from the paks with the KiteShield's scale, and a
/// strike swung at a shield-holder's raised parry is parried
#[test]
fn shield_block_collider_traces() {
    let Some(e) = env() else { return };
    let rec = std::fs::read_to_string(core_tests().join("golden/character/records.json")).unwrap();
    const KITE: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Shields/BP_KiteShield";
    let ld = mh_sim::load::load(&e.matrix, e.vfs.clone(), &[LS, KITE]).expect("load");
    let bb = ld.geo.weapons.get(KITE).and_then(|g| g.block_box).expect("the kite shield's BlockCollider");
    eprintln!("kite shield block box rel {:?} half {:?}", bb.0, bb.1);
    assert!((bb.0.z + 13.0).abs() < 1e-3 && (bb.1.y - 28.8).abs() < 1e-2 && (bb.1.z - 70.4).abs() < 1e-2);
    let mut floor = BoxWorld::new();
    floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 120.0);
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    let b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: LS.into(), left: KITE.into(), location: FVector::new(90.0, 0.0, 100.0), yaw: 180.0, ..Default::default() });
    assert!(s.left_world(b).is_some(), "the shield is held");
    let (mut parried, mut hit, mut asked) = (false, false, false);
    for n in 0..200 {
        let mut inp = Vec::new();
        if n == 1 {
            inp.push((a, SimInput { attack: Some((0, 0.0)), ..Default::default() }));
        }
        if !asked {
            if let Some(at) = s.combat.cur_m(a).and_then(|m| m.attack()) {
                if s.combat.now + s.dt as f64 >= at.windup_end + 0.1 {
                    inp.push((b, SimInput { parry: Some(0), ..Default::default() }));
                    asked = true;
                }
            }
        }
        s.step(&inp);
        let (ev, _) = s.drain();
        parried |= ev.iter().any(|x| x["kind"] == "parry");
        hit |= ev.iter().any(|x| x["kind"] == "hit");
    }
    eprintln!("shield parry: parried {parried} hit {hit}");
    assert!(parried && !hit, "parried {parried} hit {hit}");
}

/// The victim's procedural flinch (flinch.rs, fidelity-audit r8): a third-person fighter hit by a strike gets a
/// non-zero flinch that bends its spine and blends back out within a second
#[test]
fn hit_drives_procedural_flinch() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 120.0) else { return eprintln!("SKIP: no character records") };
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    let b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: LS.into(), location: FVector::new(90.0, 0.0, 100.0), yaw: 180.0, ..Default::default() });
    let mut hit_at = None;
    let mut peak = 0.0f32;
    for n in 0..240 {
        let inp = if n == 1 { vec![(a, SimInput { attack: Some((0, 0.0)), ..Default::default() })] } else { vec![] };
        s.step(&inp);
        let (ev, _) = s.drain();
        if hit_at.is_none() && ev.iter().any(|x| x["kind"] == "hit") {
            hit_at = Some(s.combat.now);
        }
        let f = &s.fanim[b].flinch;
        peak = peak.max(f.spine.0 .0.abs() + f.spine.0 .1.abs() + f.spine.0 .2.abs() + f.spine1.0 .0.abs() + f.spine1.0 .2.abs());
    }
    let f = &s.fanim[b].flinch;
    eprintln!("hit at {hit_at:?} spine idx {} target {:?} peak {peak}", f.spine_idx, f.rot_target);
    assert!(hit_at.is_some(), "the swing hit");
    assert!(peak > 2.0, "the spine flinched");
    assert!(!f.any() || f.spine.0 .0.abs() < 0.5, "and blended back out");
}

/// first-person r1: a shield has bUseEquippedOffset, so ComputeGrippedTransform rva=0x14b70f0 (decomp 191-330) attaches
/// it by EquippedOffset alone (BP_MordhauShield's, inherited by the KiteShield): the held transform = EquippedOffset in
/// the LeftWeapon socket's frame
#[test]
fn shield_attaches_by_equipped_offset() {
    let Some(e) = env() else { return };
    const KITE: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Shields/BP_KiteShield";
    let ld = mh_sim::load::load(&e.matrix, e.vfs.clone(), &[LS, KITE]).expect("load");
    let g = ld.geo.weapons.get(KITE).expect("kite shield geometry");
    let eo = g.equipped_offset.expect("bUseEquippedOffset");
    assert!((eo.loc.x + 14.69).abs() < 1e-3 && (eo.loc.y - 3.2).abs() < 1e-3 && (eo.loc.z - 1.17).abs() < 1e-3, "{:?}", eo.loc);
    assert!((eo.rot.w - 0.80552727).abs() < 1e-5 && (eo.rot.y - 0.5733062).abs() < 1e-5);
    assert!(ld.geo.weapons.get(LS).unwrap().equipped_offset.is_none(), "a longsword grips by RightHandEquipOffset / RotationOffset");
    let rec = std::fs::read_to_string(core_tests().join("golden/character/records.json")).unwrap();
    let floor = BoxWorld::new();
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 60.0);
    let b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: LS.into(), left: KITE.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    s.step(&[]);
    let w = s.left_world(b).expect("held");
    let posed = s.posed.borrow();
    let p = posed.get("B").unwrap();
    let sock = s.geo.bone_world(p, s.geo.skeleton.find("LeftWeapon").unwrap());
    let want = eo.then(&sock);
    assert!((w.loc.x - want.loc.x).abs() < 1e-3 && (w.loc.y - want.loc.y).abs() < 1e-3 && (w.loc.z - want.loc.z).abs() < 1e-3);
}

/// state/proofs/shield_motions.md: GetAttackMotionClass rva=0x14bc010 takes the left-hand weapon's profile last, so a
/// mace + kite shield fighter's RightStrike is the shield profile's BP_Shield_RightStrikeMotion (its stab the shield's
/// BP_Shield_RightStabMotion), and a fighter without a shield keeps the weapon's own motion
#[test]
fn shield_bearer_uses_shield_profile_motions() {
    let Some(e) = env() else { return };
    const KITE: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Shields/BP_KiteShield";
    const MACE: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/OneHanded/BP_Mace";
    let ld = mh_sim::load::load(&e.matrix, e.vfs.clone(), &[MACE, KITE]).expect("load");
    let rec = std::fs::read_to_string(core_tests().join("golden/character/records.json")).unwrap();
    let mut floor = BoxWorld::new();
    floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 60.0);
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: MACE.into(), left: KITE.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    let b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: MACE.into(), location: FVector::new(500.0, 0.0, 100.0), ..Default::default() });
    let w = &s.combat;
    let short = |p: Option<String>| p.unwrap_or_default().rsplit('/').next().unwrap_or("").to_string();
    assert_eq!(short(w.attack_motion_bp(a, mordhau_core::combat::enums::mv::RIGHT_STRIKE)), "BP_Shield_RightStrikeMotion");
    assert_eq!(short(w.attack_motion_bp(a, mordhau_core::combat::enums::mv::STAB)), "BP_Shield_RightStabMotion");
    assert_ne!(short(w.attack_motion_bp(b, mordhau_core::combat::enums::mv::RIGHT_STRIKE)), "BP_Shield_RightStrikeMotion");
    s.step(&[(a, SimInput { attack: Some((0, 0.0)), ..Default::default() })]);
    assert!(s.combat.cur_m(a).map(|m| m.bp.ends_with("BP_Shield_RightStrikeMotion")).unwrap_or(false));
}

/// fp-anim r1 (state/proofs/fp_anim_items.md item 1): a first-person character's upper blend-space Velocity is
/// quantized (NativeUpdateAnimation decomp 2443-2522): 60 while walking from the first moving frame, 90 with sprint
/// held, 0 standing; MovementSpeedScale rises above 1 only while sprinting (AnimRateFactor1PMaxSprint 0.15). The same
/// walk in third person ramps up through intermediate values.
#[test]
fn first_person_upper_velocity_is_quantized() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 60.0) else { return eprintln!("SKIP: no character records") };
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    let b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: LS.into(), location: FVector::new(0.0, 500.0, 100.0), ..Default::default() });
    s.set_first_person(a, true);
    s.set_view_target(a, true, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
    s.set_server_pose(a, false);
    s.set_server_pose(b, false);
    for _ in 0..30 {
        s.step(&[]);
    }
    assert_eq!(s.fanim[a].upper_input.1, 0.0);
    let (mut fp_vals, mut moved) = (Vec::new(), 0);
    for _ in 0..150 {
        s.step(&[(a, SimInput { fwd: 1.0, ..Default::default() }), (b, SimInput { fwd: 1.0, ..Default::default() })]);
        let sp = |fi: usize| { let v = s.movers[fi].velocity; ((v.x * v.x + v.y * v.y) as f64).sqrt() };
        if sp(a) > 1.0 {
            moved += 1;
            fp_vals.push(s.fanim[a].upper_input.1);
        }
        if sp(b) > 1.0 {
            // third person: the ramp clamp(2 speed / MaxWalkSpeed, 0, 1) * 60 below the walk speed
            let w = s.movers[b].c.max_walk_speed as f64;
            let sb = sp(b) as f32 as f64;
            if sb <= w {
                let want = ((2.0 / w as f32) * sb as f32).clamp(0.0, 1.0) as f64 * 60.0;
                assert!((s.fanim[b].upper_input.1 - want).abs() < 1e-3, "3P ramp {} vs {want}", s.fanim[b].upper_input.1);
            }
        }
    }
    assert!(moved > 20, "the fighter walked");
    assert!(fp_vals.iter().all(|v| *v == 60.0), "1P walk = 60: {fp_vals:?}");
    let mut scale = 1.0;
    for _ in 0..240 {
        s.step(&[(a, SimInput { fwd: 1.0, sprint: true, ..Default::default() })]);
        scale = s.fanim[a].movement_speed_scale;
    }
    assert_eq!(s.fanim[a].upper_input.1, 90.0);
    assert!(scale > 1.0 && scale <= 1.15 + 1e-9, "sprint rate factor {scale}");
}

/// fp-anim r1 (proof item 4): R on a weapon with an alternate mode plays its ModeSwitchAnimation montage and the anim
/// instance switches to the Second* 1P blend space (SwitchMode_Implementation swap, UpdateEquipmentData)
#[test]
fn mode_switch_plays_montage_and_uses_second_blend_space() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 120.0) else { return eprintln!("SKIP: no character records") };
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    s.set_first_person(a, true);
    s.set_view_target(a, true, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
    for _ in 0..10 {
        s.step(&[]);
    }
    let before = s.fanim[a].upper_bs.as_ref().map(|b| b.path.clone()).unwrap_or_default();
    s.step(&[(a, SimInput { switch_mode: true, ..Default::default() })]);
    // fp-anim r2: the montage starts with the motion, the assets swap at FinishSwitch (StartTime + 0.15)
    for _ in 0..5 {
        s.step(&[]);
    }
    let early = s.fanim[a].upper_bs.as_ref().map(|b| b.path.clone()).unwrap_or_default();
    assert_eq!(early, before, "not swapped before StartTime + 0.15");
    for _ in 0..20 {
        s.step(&[]);
    }
    assert!(s.combat.fighters[a].alternate_mode, "the Longsword has an alternate mode");
    let after = s.fanim[a].upper_bs.as_ref().map(|b| b.path.clone()).unwrap_or_default();
    let live: Vec<String> = s.fanim[a].ma.montages.insts.iter().filter(|i| !i.stopped).map(|i| i.asset.path.rsplit('/').next().unwrap_or("").to_string()).collect();
    eprintln!("1P upper bs {before} -> {after}; montages {live:?}");
    assert!(before.ends_with("BS_2H_Sword_Locomotion_1P") && after.ends_with("BS_2H_Polearm_Locomotion_1P"));
    assert!(live.iter().any(|p| p.starts_with("MTG_2H_Sword_ToMordhau_Idle")), "{live:?}");
}

/// fp-anim r2 (proof items 5-7): in first person with the Kite shield, ModifyBone_188 moves the LeftShoulder by the
/// shield's LeftShoulderIdleOffset1P (10, 0, -5) in component space at idle; the feet go to their virtual bones offset
/// by (0, 25, 0) (TwoBoneIK_7 / _8); the hand-spring target is 1 for a melee main equipment
#[test]
fn first_person_shield_shoulder_and_feet() {
    const KITE: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Shields/BP_KiteShield";
    let Some(e) = env() else { return };
    let Ok(rec) = std::fs::read_to_string(core_tests().join("golden/character/records.json")) else { return eprintln!("SKIP: no character records") };
    let ld = mh_sim::load::load(&e.matrix, e.vfs.clone(), &[LS, KITE]).expect("load");
    let mut floor = BoxWorld::new();
    floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), CharRecords(&rec).load().unwrap(), Box::new(floor), 1.0 / 60.0);
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), left: KITE.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    for _ in 0..10 {
        s.step(&[]);
    }
    assert_eq!(s.fanim[a].left_shoulder_idle_1p, FVector::new(10.0, 0.0, -5.0));
    assert_eq!(s.fanim[a].proc.hand_spring_target, 1.0);
    let sk = s.geo.skeleton.clone();
    let ls = sk.find("LeftShoulder").unwrap();
    let lf = sk.find("LeftFoot").unwrap();
    let cs3 = sk.to_component(&s.local_pose(a));
    s.set_first_person(a, true);
    s.set_view_target(a, true, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
    for _ in 0..10 {
        s.step(&[]);
    }
    let cs1 = sk.to_component(&s.local_pose(a));
    let vb = s.fanim[a].lower.as_ref().and_then(|l| l.vb_feet).expect("1P virtual-bone feet");
    eprintln!("left shoulder 3P {:?} 1P {:?}; left foot 1P {:?} vb {:?}", cs3[ls].loc, cs1[ls].loc, cs1[lf].loc, vb.0);
    // EVD_MOV_003: after the feet IK, the 1P grounding (ModifyBone_97 / _96, nodes 445 / 449) lowers the thighs by
    // RootTranslationOffset (world Z; the mesh only yaws) x GroundingWeight
    let g = &s.fanim[a].grounding;
    let target = vb.0 + FVector::new(0.0, 0.0, g.root_translation_offset.z * g.weight);
    assert!((cs1[lf].loc - target).length() < 1.0, "the foot reaches its VB target (+ the grounding offset)");
    assert!(s.fanim[a].left_shoulder_idle_1p.length() > 0.0);
}

/// fp-anim r4: AMordhauCharacter::CanJumpInternal_Implementation rva=0x1532630 refuses a jump while the current attack is
/// between WindupEnd and WindupEnd + ReleaseJumpBlockTime (the Longsword strike's Blueprint value); before the attack,
/// and after the window, the jump goes through
#[test]
fn jump_blocked_early_in_release() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 60.0) else { return eprintln!("SKIP: no character records") };
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    for _ in 0..90 {
        s.step(&[]);
    }
    s.step(&[(a, SimInput { attack: Some((0, 0.0)), ..Default::default() })]);
    let we = s.combat.cur_m(a).and_then(|m| m.attack()).map(|x| x.windup_end).expect("attack");
    while s.combat.now < we + 0.05 {
        s.step(&[]);
    }
    s.step(&[(a, SimInput { jump: true, ..Default::default() })]);
    s.step(&[]);
    assert!(!s.movers[a].is_airborne(), "jumped inside the release jump block window");
    for _ in 0..120 {
        s.step(&[]);
    }
    s.step(&[(a, SimInput { jump: true, ..Default::default() })]);
    s.step(&[]);
    assert!(s.movers[a].is_airborne(), "the jump after the window");
}

/// reader method: a parried strike's BlockedMotion lasts EndTime - StartTime = 0.35 s or 0.55 s in the real game
/// (state/live_rec/everything.json, 20 parried greatsword strikes). Print ours.
#[test]
#[ignore]
fn probe_parried_blocked_duration() {
    let Some(e) = env() else { return };
    let Some(mut s) = sim(&e, 1.0 / 120.0) else { return };
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    let b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: LS.into(), location: FVector::new(70.0, 0.0, 100.0), yaw: 180.0, ..Default::default() });
    let mut asked = false;
    let mut seen = std::collections::HashSet::new();
    for n in 0..300 {
        let mut inp = Vec::new();
        if n == 1 {
            inp.push((a, SimInput { attack: Some((0, 0.0)), ..Default::default() }));
        }
        if !asked {
            if let Some(at) = s.combat.cur_m(a).and_then(|m| m.attack()) {
                if s.combat.now + s.dt as f64 >= at.windup_end + 0.15 {
                    inp.push((b, SimInput { parry: Some(0), ..Default::default() }));
                    asked = true;
                }
            }
        }
        s.step(&inp);
        if let Some(m) = s.combat.cur_m(a) {
            if m.kind() == "Blocked" && seen.insert(m.start_time.to_bits()) {
                let bl = m.blocked().unwrap();
                let bd = m.def.blocked.as_ref().unwrap();
                println!("  net param2 {} (t = x 0.025), offset {} limits {:?} chambered {:?}", s.combat.fighters[a].net.param2, bd.parried_recovery_time_offset, bd.parried_recovery_time_limits, bd.chambered_recovery_time_limits);
                println!("A Blocked: reason {} start {:.4} end {:.4} -> {:.4} s (stun {})", bl.reason, m.start_time, m.end_time, m.end_time - m.start_time, bl.b_is_stun);
            }
        }
        let _ = s.drain();
    }
}

/// EVD_SWG_004: what drives bHasHitIncludingCosmeticHit (the hit-effect IK). On the authority it is set only with
/// bHasHit (UAttackMotion::OnDynamicParamChanged rva=0x1631060, decomp 7361-7364): OnLateTick's cosmetic-hit path
/// (decomp 5440-5475) requires a non-authority owner, and SetHasHitIncludingCosmeticHit (rva=0x163dff0) is reached only
/// through its exec thunk (UAttackMotion::execSetHasHitIncludingCosmeticHit rva=0x16819e0, the call at 0x141681a52) with
/// no Blueprint caller in the extracted assets. So a body hit holds the hand (weight 1 from the hit) and a parried
/// swing does not (it goes straight to Blocked). The reader record agrees: all 24 hit-effect onsets coincide with
/// EndTime -> ReleaseEnd + HitRecovery (the bHasHit branch), the later Blocked being the blade reaching the floor.
#[test]
fn hit_effect_ik_follows_a_body_hit_not_a_parry() {
    let Some(e) = env() else { return };
    for parry in [false, true] {
        let Some(mut s) = sim(&e, 1.0 / 120.0) else { return eprintln!("SKIP: no character records") };
        s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
        let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: LS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
        let b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: LS.into(), location: FVector::new(70.0, 0.0, 100.0), yaw: 180.0, ..Default::default() });
        s.set_first_person(a, true);
        s.set_view_target(a, true, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
        let (mut asked, mut contact, mut w_after) = (false, None, None);
        for n in 0..200 {
            let mut inp = Vec::new();
            if n == 1 {
                inp.push((a, SimInput { attack: Some((0, 0.0)), ..Default::default() }));
            }
            if parry && !asked {
                if let Some(at) = s.combat.cur_m(a).and_then(|m| m.attack()) {
                    if s.combat.now + s.dt as f64 >= at.windup_end + 0.15 {
                        inp.push((b, SimInput { parry: Some(0), ..Default::default() }));
                        asked = true;
                    }
                }
            }
            s.step(&inp);
            if let Some(k) = contact {
                if w_after.is_none() && n == k + 1 {
                    w_after = Some(s.fanim[a].hit_effect.weight);
                }
            }
            let (ev, _) = s.drain();
            if contact.is_none() && ev.iter().any(|x| x["kind"] == "hit" || x["kind"] == "parry") {
                contact = Some(n);
            }
        }
        let w = w_after.expect("no contact");
        println!("parry {parry}: hit-effect weight one step after the contact {w}");
        if parry {
            assert_eq!(w, 0.0, "a parried swing should not start the hit-effect IK on the authority");
        } else {
            assert_eq!(w, 1.0, "a body hit should hold the hand (hit-effect weight 1)");
        }
    }
}

/// the timing probes' step: MH_TIMING_HZ (default 240, the reader record's frame rate)
fn hz_dt() -> f32 {
    1.0 / std::env::var("MH_TIMING_HZ").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(240.0)
}

const GS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Greatsword";

/// EVD_SWG_005+: a greatsword duel scenario; `script(t, sim, a, b)` gives the inputs at sim time t (s) for `secs`.
/// Returns each fighter's non-idle motions as (kind, StartTime, EndTime).
fn gs_timeline(e: &Env, dist: f32, secs: f64, script: &dyn Fn(f64, &Sim, usize, usize) -> Vec<(usize, SimInput)>) -> Option<Vec<Vec<(String, f64, f64)>>> {
    let rec = std::fs::read_to_string(core_tests().join("golden/character/records.json")).ok()?;
    let records = CharRecords(&rec).load().unwrap();
    let ld = mh_sim::load::load(&e.matrix, e.vfs.clone(), &[GS]).expect("load");
    let mut floor = BoxWorld::new();
    floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
    let mut s = Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), records, Box::new(floor), hz_dt());
    s.enable_anim(mh_sim::animgraph::AnimAssets::new(e.vfs.clone()).expect("anim assets"));
    let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: GS.into(), location: FVector::new(0.0, 0.0, 100.0), ..Default::default() });
    let b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: GS.into(), location: FVector::new(dist, 0.0, 100.0), yaw: 180.0, ..Default::default() });
    s.set_first_person(a, true);
    s.set_view_target(a, true, false); // Local 1P probe; 3P cases retain their remote-pose baseline.
    let mut seen: Vec<Vec<(String, f64, f64)>> = vec![vec![], vec![]];
    while s.combat.now < secs {
        let inp = script(s.combat.now, &s, a, b);
        s.step(&inp);
        for (k, &fi) in [a, b].iter().enumerate() {
            if let Some(m) = s.combat.cur_m(fi) {
                if m.kind() == "Idle" {
                    continue;
                }
                let wu = m.attack().map(|x| format!(" w{:.3} ty{} hit{}", x.windup_end - m.start_time, x.ty, x.b_has_hit as u8)).unwrap_or_default();
                let kind = format!("{} mr{} cb{} ca{}{wu}", m.kind(), m.movement_restriction, m.b_can_block as u8, m.b_can_attack as u8);
                match seen[k].iter_mut().find(|x| x.1 == m.start_time && x.0.starts_with(m.kind())) {
                    Some(x) => {
                        x.2 = m.end_time;
                        x.0 = kind;
                    }
                    None => seen[k].push((kind, m.start_time, m.end_time)),
                }
            }
        }
        let (ev, _) = s.drain();
        for x in ev {
            if std::env::var("MH_TIMING_EVENTS").is_ok() {
                println!("  event @{:.3} {:?}", s.combat.now, x.get("kind"));
            }
        }
    }
    Some(seen)
}

/// once-only input at the first step at or after `at` seconds
fn once(t: f64, at: f64, dt: f32) -> bool {
    t >= at && t < at + dt as f64
}

#[test]
#[ignore]
fn probe_greatsword_timing_table() {
    let Some(e) = env() else { return };
    let dt = hz_dt();
    use mordhau_core::combat::enums::mv;
    let atk = |fi: usize, m: i64| vec![(fi, SimInput { attack: Some((m, 0.0)), ..Default::default() })];
    let show = |name: &str, t: Option<Vec<Vec<(String, f64, f64)>>>| {
        let t = t.unwrap();
        let t0 = t[0].first().map(|m| m.1).unwrap_or(0.0);
        for (who, ms) in ["A", "B"].iter().zip(t.iter()) {
            for m in ms {
                println!("{name:20} {who} {:40} start +{:.3} dur {:.3}", m.0, m.1 - t0, m.2 - m.1);
            }
        }
    };
    show("strike miss", gs_timeline(&e, 600.0, 3.0, &|t, _, a, _| if once(t, 0.1, dt) { atk(a, mv::RIGHT_STRIKE) } else { vec![] }));
    show("stab miss", gs_timeline(&e, 600.0, 3.0, &|t, _, a, _| if once(t, 0.1, dt) { atk(a, mv::STAB) } else { vec![] }));
    show("strike hit", gs_timeline(&e, 80.0, 3.0, &|t, _, a, _| if once(t, 0.1, dt) { atk(a, mv::RIGHT_STRIKE) } else { vec![] }));
    show("strike hit + combo", gs_timeline(&e, 80.0, 4.0, &|t, s, a, _| {
        let hit = s.combat.cur_m(a).and_then(|m| m.attack()).is_some_and(|x| x.b_has_hit);
        if once(t, 0.1, dt) { atk(a, mv::RIGHT_STRIKE) } else if hit { atk(a, mv::LEFT_STRIKE) } else { vec![] }
    }));
    show("feint @0.40", gs_timeline(&e, 600.0, 3.0, &|t, _, a, _| if once(t, 0.1, dt) { atk(a, mv::RIGHT_STRIKE) } else if once(t, 0.5, dt) { vec![(a, SimInput { feint: true, ..Default::default() })] } else { vec![] }));
    // looking down: the blade hits B, then the floor (the record's 0.35 s Hit-reason blocks)
    show("hit then floor", gs_timeline(&e, 80.0, 3.0, &|t, _, a, _| {
        let mut v = vec![(a, SimInput { look_up: Some(-70.0), ..Default::default() })];
        if once(t, 0.1, dt) {
            v[0].1.attack = Some((mv::RIGHT_STRIKE, 0.0));
        }
        v
    }));
    show("miss into floor", gs_timeline(&e, 600.0, 3.0, &|t, _, a, _| {
        let mut v = vec![(a, SimInput { look_up: Some(-70.0), ..Default::default() })];
        if once(t, 0.1, dt) {
            v[0].1.attack = Some((mv::RIGHT_STRIKE, 0.0));
        }
        v
    }));
    show("stab hit", gs_timeline(&e, 110.0, 3.0, &|t, _, a, _| if once(t, 0.1, dt) { atk(a, mv::STAB) } else { vec![] }));
    // B parries A's strike, then strikes back 0.1 s after the block (a riposte)
    show("riposte", gs_timeline(&e, 80.0, 4.0, &|t, s, a, b| {
        let mut v = if once(t, 0.1, dt) { atk(a, mv::RIGHT_STRIKE) } else { vec![] };
        if let Some(at) = s.combat.cur_m(a).and_then(|m| m.attack()) {
            if once(t, at.windup_end + 0.1, dt) {
                v.push((b, SimInput { parry: Some(0), ..Default::default() }));
            }
        }
        if s.combat.cur_m(a).is_some_and(|m| m.kind() == "Blocked") && s.combat.cur_m(b).is_some_and(|m| m.kind() == "Parry") {
            v.push((b, SimInput { attack: Some((mv::RIGHT_STRIKE, 0.0)), ..Default::default() }));
        }
        v
    }));
    show("parry miss", gs_timeline(&e, 600.0, 3.0, &|t, _, _, b| if once(t, 0.1, dt) { vec![(b, SimInput { parry: Some(0), ..Default::default() })] } else { vec![] }));
    show("parry success", gs_timeline(&e, 80.0, 3.0, &|t, s, a, b| {
        let mut v = if once(t, 0.1, dt) { atk(a, mv::RIGHT_STRIKE) } else { vec![] };
        if let Some(at) = s.combat.cur_m(a).and_then(|m| m.attack()) {
            if once(t, at.windup_end + 0.1, dt) {
                v.push((b, SimInput { parry: Some(0), ..Default::default() }));
            }
        }
        v
    }));
}

/// EVD_SWG_005..013: the greatsword timings of the reader record (everything.json, 124 motion instances; real values
/// in the comments) against ours at the record's 240 Hz: (kind, StartTime offset, EndTime - StartTime, windup).
#[test]
fn greatsword_timings_match_the_record() {
    let Some(e) = env() else { return };
    let dt = 1.0 / 240.0;
    use mordhau_core::combat::enums::mv;
    let atk = |fi: usize, m: i64| vec![(fi, SimInput { attack: Some((m, 0.0)), ..Default::default() })];
    let close = |a: f64, b: f64, tol: f64, what: &str| assert!((a - b).abs() <= tol, "{what}: ours {a:.3}, record {b:.3}");
    let dur = |t: &Vec<Vec<(String, f64, f64)>>, who: usize, k: usize| t[who][k].2 - t[who][k].1;
    // 005 strike miss: EndTime = WindupEnd + Release + MissRecovery = 1.775 (26 / 28 planned)
    let t = gs_timeline(&e, 600.0, 3.0, &|t, _, a, _| if once(t, 0.1, dt) { atk(a, mv::RIGHT_STRIKE) } else { vec![] }).unwrap();
    close(dur(&t, 0, 0), 1.775, 0.002, "strike miss");
    // 006 strike hit: EndTime -> ReleaseEnd + HitRecovery = 1.475 (19 / 28 final)
    let t = gs_timeline(&e, 80.0, 3.0, &|t, _, a, _| if once(t, 0.1, dt) { atk(a, mv::RIGHT_STRIKE) } else { vec![] }).unwrap();
    close(dur(&t, 0, 0), 1.475, 0.002, "strike hit");
    // 007 stab miss 1.732 (3 / 3 planned), stab hit 1.432 (1 final)
    let t = gs_timeline(&e, 600.0, 3.0, &|t, _, a, _| if once(t, 0.1, dt) { atk(a, mv::STAB) } else { vec![] }).unwrap();
    close(dur(&t, 0, 0), 1.732, 0.002, "stab miss");
    let t = gs_timeline(&e, 110.0, 3.0, &|t, _, a, _| if once(t, 0.1, dt) { atk(a, mv::STAB) } else { vec![] }).unwrap();
    close(dur(&t, 0, 0), 1.432, 0.002, "stab hit");
    // 008 combo after a hit: the next strike starts at ReleaseEnd (record +1.078), windup 0.575 + ComboWindupIncrease
    // 0.2 (record CounterCompensate onset +0.777), planned 1.975
    let t = gs_timeline(&e, 80.0, 4.0, &|t, s, a, _| {
        let hit = s.combat.cur_m(a).and_then(|m| m.attack()).is_some_and(|x| x.b_has_hit);
        if once(t, 0.1, dt) { atk(a, mv::RIGHT_STRIKE) } else if hit { atk(a, mv::LEFT_STRIKE) } else { vec![] }
    })
    .unwrap();
    close(t[0][1].1 - t[0][0].1, 1.078, 0.006, "combo start");
    close(dur(&t, 0, 1), 1.975, 0.002, "combo planned");
    // 009 feinted at +0.40: FeintedMotion 0.336 / 0.338
    let t = gs_timeline(&e, 600.0, 3.0, &|t, _, a, _| if once(t, 0.1, dt) { atk(a, mv::RIGHT_STRIKE) } else if once(t, 0.5, dt) { vec![(a, SimInput { feint: true, ..Default::default() })] } else { vec![] }).unwrap();
    close(dur(&t, 0, 1), 0.337, 0.003, "feinted");
    // 010 missed parry 1.000 - 1.004
    let t = gs_timeline(&e, 600.0, 3.0, &|t, _, _, b| if once(t, 0.1, dt) { vec![(b, SimInput { parry: Some(0), ..Default::default() })] } else { vec![] }).unwrap();
    close(dur(&t, 1, 0), 1.002, 0.004, "parry miss");
    // 011 successful parry 0.775 - 0.779; 012 the parried attacker's Blocked: MovementRestriction 2 as the record's
    // 0.55 s group (its length = the defender's fastest windup - 0.05 clamped to 0.55: 0.525 for a greatsword defender;
    // the record's bots' weapons are unknown); 013 a riposte keeps the 0.575 windup (record 0.567 - 0.579)
    let t = gs_timeline(&e, 80.0, 4.0, &|t, s, a, b| {
        let mut v = if once(t, 0.1, dt) { atk(a, mv::RIGHT_STRIKE) } else { vec![] };
        if let Some(at) = s.combat.cur_m(a).and_then(|m| m.attack()) {
            if once(t, at.windup_end + 0.1, dt) {
                v.push((b, SimInput { parry: Some(0), ..Default::default() }));
            }
        }
        v
    })
    .unwrap();
    close(dur(&t, 1, 0), 0.777, 0.004, "parry success");
    assert!(t[0][1].0.starts_with("Blocked mr2"), "{}", t[0][1].0);
    close(dur(&t, 0, 1), 0.525, 0.002, "parried Blocked (greatsword defender)");
    let t = gs_timeline(&e, 80.0, 4.0, &|t, s, a, b| {
        let mut v = if once(t, 0.1, dt) { atk(a, mv::RIGHT_STRIKE) } else { vec![] };
        if let Some(at) = s.combat.cur_m(a).and_then(|m| m.attack()) {
            if once(t, at.windup_end + 0.1, dt) {
                v.push((b, SimInput { parry: Some(0), ..Default::default() }));
            }
        }
        if s.combat.cur_m(a).is_some_and(|m| m.kind() == "Blocked") && s.combat.cur_m(b).is_some_and(|m| m.kind() == "Parry") {
            v.push((b, SimInput { attack: Some((mv::RIGHT_STRIKE, 0.0)), ..Default::default() }));
        }
        v
    })
    .unwrap();
    assert!(t[1][1].0.contains("w0.575 ty1"), "riposte {}", t[1][1].0);
    // 014 flinch 0.900 (9 / 9)
    let t = gs_timeline(&e, 80.0, 3.0, &|t, _, a, _| {
        let mut v = vec![(a, SimInput { look_up: Some(-70.0), ..Default::default() })];
        if once(t, 0.1, dt) {
            v[0].1.attack = Some((mv::RIGHT_STRIKE, 0.0));
        }
        v
    })
    .unwrap();
    close(dur(&t, 1, 0), 0.900, 0.002, "flinch");
}

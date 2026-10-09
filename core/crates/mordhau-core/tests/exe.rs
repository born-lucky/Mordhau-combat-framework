//! Exe-exact mode tests (World::new, Precision::Exe; combat/exe.rs lists every difference from the reference).
//!
//! 1. Unit rules of the exe model: the f32 clock (UWorld::Tick TimeSeconds += DeltaSeconds in float), f32 records,
//!    UMordhauMotion::Tick's OnEnded without a current-motion test, the stamina byte's RoundToInt.
//! 2. The golden scenarios replayed in exe mode with the exe's f32 records (RecordsJsonExe): the same motion sequence
//!    (fighter, motion kind) as the reference, each change within one tick of the reference's time; every scenario's
//!    drift (max |t_exe - t_ref| of motion changes, final health/stamina) is printed. Needs the git-ignored golden data
//!    (SKIP without it unless MORDHAU_GOLDEN_REQUIRED=1).

use mordhau_core::combat::world::{Call, Input};
use mordhau_core::combat::World;
use mordhau_core::data::{decode_exact, RecordsJsonExe, SpecSource};
use serde_json::Value;
use std::path::PathBuf;
use std::rc::Rc;

fn tests_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests")
}

fn spec() -> Option<Rc<mordhau_core::data::Spec>> {
    let txt = std::fs::read_to_string(tests_dir().join("golden/spec.json")).ok()?;
    Some(Rc::new(RecordsJsonExe(&txt).load_spec().expect("spec")))
}

const LS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";

#[test]
fn exe_clock_is_f32_accumulated() {
    let Some(sp) = spec() else { return eprintln!("SKIP: no golden spec") };
    let mut w = World::new(sp, 1.0 / 120.0);
    for _ in 0..1000 {
        w.step();
    }
    // TimeSeconds += DeltaSeconds in binary32 drifts from n * dt
    let mut t = 0f32;
    let dt = (1.0f64 / 120.0) as f32;
    for _ in 0..1000 {
        t += dt;
    }
    assert_eq!(w.now, t as f64);
    assert_ne!(w.now, 1000.0 * (1.0 / 120.0));
}

#[test]
fn exe_records_are_f32() {
    let Some(sp) = spec() else { return eprintln!("SKIP: no golden spec") };
    let ls = sp.weapon(LS).unwrap();
    assert_eq!(ls.strike.windup, ls.strike.windup as f32 as f64);
    let mut w = World::new(sp.clone(), 0.002);
    let a = w.add_fighter("A", LS, "");
    w.at(0.1, Input::Attack { who: "A".into(), mv: 0, angle: 0.0 });
    w.run_until(0.2);
    let m = w.cur_m(a).unwrap();
    let at = m.attack().unwrap();
    // every timeline value is a binary32
    for v in [m.start_time, at.windup_end, at.release_end, m.end_time, at.ai.windup] {
        assert_eq!(v, v as f32 as f64, "{v} is not an f32");
    }
}

#[test]
fn exe_stamina_byte_round_to_int() {
    let Some(sp) = spec() else { return eprintln!("SKIP: no golden spec") };
    let mut w = World::new(sp, 0.01);
    let a = w.add_fighter("A", LS, "");
    for s in 0..=100 {
        w.fighters[a].stamina = s;
        assert_eq!(w.stamina_byte(a), s, "stamina {s}");
    }
}

fn s(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or("").to_string()
}
fn f(v: &Value, k: &str, d: f64) -> f64 {
    v[k].as_f64().unwrap_or(d)
}
fn i(v: &Value, k: &str, d: i64) -> i64 {
    v[k].as_f64().map(|x| x as i64).unwrap_or(d)
}

fn input_of(ev: &Value, alias: &dyn Fn(&str) -> String) -> Input {
    let who = s(ev, "who");
    match ev["kind"].as_str().unwrap() {
        "attack" => Input::Attack { who, mv: i(ev, "move", 0), angle: f(ev, "angle", 0.0) },
        "feint" => Input::Feint { who },
        "parry" => Input::Parry { who, bt: i(ev, "bt", 0) },
        "release_block" => Input::ReleaseBlock { who },
        "switch_mode" => Input::SwitchMode { who },
        "toggle_mode" => Input::ToggleMode { who },
        "contact" => Input::Contact { who, target: s(ev, "target"), bone: ev["bone"].as_str().unwrap_or("Spine1").into() },
        "set" => {
            let v = &ev["value"];
            Input::Call(match s(ev, "field").as_str() {
                "stamina" => Call::SetStamina { who, v: v.as_f64().unwrap() as i64 },
                "health" => Call::SetHealth { who, v: v.as_f64().unwrap() as i64 },
                "armor_tier_override" => Call::SetArmorTierOverride { who, v: v.as_f64().unwrap() as i64 },
                "airborne" => Call::SetAirborne { who, v: v.as_bool().unwrap() },
                "holding_block" => Call::SetHoldingBlock { who, v: v.as_bool().unwrap() },
                "preset_flip" => Call::SetPresetFlip { who, v: v.as_bool().unwrap() },
                "airborne_time" => Call::SetAirborneTime { who, v: v.as_f64().unwrap() },
                "look_up_value" => Call::SetLookUp { who, v: v.as_f64().unwrap() },
                x => panic!("set {x}"),
            })
        }
        "weapon_no_drop" => Input::Call(Call::WeaponNoDrop { who }),
        "blocked" => Input::Call(Call::Blocked { who, reason: i(ev, "reason", 0), flags: i(ev, "flags", 0), time: f(ev, "time", 0.0) }),
        "request_parry" => Input::Call(Call::RequestParry { who, bt: i(ev, "bt", 0), ftp: ev["ftp"].as_bool().unwrap_or(true) }),
        "preset_attack" => Input::Call(Call::PresetAttack { who, mv: i(ev, "move", 0), angle: f(ev, "angle", 0.0) }),
        "attack_now" => Input::Call(Call::AttackNow { who, mv: i(ev, "move", 0), angle: f(ev, "angle", 0.0) }),
        "equip_right" => Input::Call(Call::EquipRight { who, weapon: alias(&s(ev, "weapon")) }),
        k => panic!("unknown input kind {k}"),
    }
}

/// (t, who, motion) of every "motion" event
fn motions_of(evs: &[(f64, String, String)]) -> Vec<(f64, String, String)> {
    evs.to_vec()
}

#[test]
fn exe_golden_scenarios_same_motion_sequence() {
    let dir = tests_dir();
    let required = std::env::var("MORDHAU_GOLDEN_REQUIRED").map(|v| v == "1").unwrap_or(false);
    let Some(sp) = spec() else {
        assert!(!required, "golden spec missing");
        return eprintln!("SKIP: no golden spec");
    };
    let doc: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("scenarios/combat.json")).unwrap()).unwrap();
    let aliases = doc["weapons"].clone();
    let alias = move |x: &str| -> String { aliases[x].as_str().unwrap_or(x).to_string() };
    let mut fails = Vec::new();
    for sc0 in doc["scenarios"].as_array().unwrap() {
        let name = s(sc0, "name");
        let Ok(golden) = std::fs::read_to_string(dir.join("golden/combat").join(format!("{name}.jsonl"))) else { continue };
        let mut lines = golden.lines();
        let mut header: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        decode_exact(&mut header);
        let sc = if header["parsed"].is_object() { header["parsed"].clone() } else { sc0.clone() };
        // reference motion changes + final state
        let mut ref_m = Vec::new();
        let mut last = Value::Null;
        for l in lines {
            let mut row: Value = serde_json::from_str(l).unwrap();
            decode_exact(&mut row);
            for e in row["ev"].as_array().unwrap() {
                if e["kind"] == "motion" {
                    ref_m.push((e["t"].as_f64().unwrap(), s(e, "who"), s(e, "motion")));
                }
            }
            last = row;
        }
        // exe run
        let dt = f(&sc, "dt", 0.001);
        let mut w = World::new(sp.clone(), dt);
        if sc.get("mode").is_some() {
            w.set_game_mode(&s(&sc["mode"], "game_mode"), &s(&sc["mode"], "game_state"));
        }
        for fd in sc["fighters"].as_array().unwrap() {
            let fi = w.add_fighter(&s(fd, "name"), &alias(&s(fd, "weapon")), &alias(&s(fd, "left")));
            if fd.get("team").is_some() {
                w.fighters[fi].team = i(fd, "team", 255);
            }
        }
        for ev in sc["inputs"].as_array().unwrap() {
            w.at(f(ev, "t", 0.0), input_of(ev, &alias));
        }
        let until = f(&sc, "until", 0.0);
        let n_ticks = (until / dt).round() as i64;
        while w.tick_n < n_ticks {
            w.step();
        }
        let exe_m: Vec<(f64, String, String)> =
            w.hits.iter().filter(|e| e["kind"] == "motion").map(|e| (e["t"].as_f64().unwrap(), e["who"].as_str().unwrap().to_string(), e["motion"].as_str().unwrap().to_string())).collect();
        let (rm, em) = (motions_of(&ref_m), motions_of(&exe_m));
        let seq_r: Vec<_> = rm.iter().map(|x| (&x.1, &x.2)).collect();
        let seq_e: Vec<_> = em.iter().map(|x| (&x.1, &x.2)).collect();
        let mut drift = 0f64;
        let mut ok = seq_r == seq_e;
        if ok {
            for (a, b) in rm.iter().zip(&em) {
                drift = drift.max((a.0 - b.0).abs());
            }
            ok = drift <= dt * 1.5;
        }
        let fin: Vec<String> = w
            .fighters
            .iter()
            .map(|fi| {
                let g = &last["f"][&fi.name];
                format!("{} hp {}/{} st {}/{}", fi.name, fi.health, g["health"], fi.stamina, g["stamina"])
            })
            .collect();
        eprintln!("{} {name}: {} motion changes, max drift {:.6} s; {}", if ok { "SAME" } else { "DIFF" }, em.len(), drift, fin.join(", "));
        // Explained divergences (the exe's own outcome, not a port error):
        //  shield_wall_kite: B's contact is scheduled at tick 450 of 2 ms = exactly StartTime(0.1) + ShieldWallRaiseTime
        //  (0.8). The exe's float clock reads 0.8999955 there, so UParryMotion::CheckParry rva=0x164ed40's
        //  `TimeSeconds < ParryUpTime + StartTime` still refuses the shield wall and the hit lands (A flinches); the
        //  reference's n * dt clock reads 0.9 and the parry succeeds.
        let explained = ["shield_wall_kite"];
        if !ok && explained.contains(&name.as_str()) {
            eprintln!("EXPLAINED {name}");
        } else if !ok {
            let k = seq_r.iter().zip(&seq_e).position(|(a, b)| a != b).unwrap_or(seq_r.len().min(seq_e.len()));
            fails.push(format!("{name}: first difference at change {k}: ref {:?} vs exe {:?}", rm.get(k), em.get(k)));
        }
    }
    for x in &fails {
        eprintln!("EXE-DIFF {x}");
    }
    assert!(fails.is_empty(), "exe mode diverges from the reference beyond one tick:\n{}", fails.join("\n"));
}

/// combat r13's regression check, mirrored: UParryMotion::CheckParry rva=0x164ed40 (decomp 1124-1129) blocks only when
/// (ParryMask | 2 when neither held nor shield wall) & the attack's AttackMask != 0, so a held Fists parry (ParryMask 4)
/// does not block a Longsword (AttackMask 1), while Fists vs Fists completes the parry (attacker Blocked).
#[test]
fn fists_parry_masks() {
    let Some(sp) = spec() else { return eprintln!("SKIP: no golden spec") };
    const FISTS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/BP_FistsWeapon";
    for (attacker, blocks) in [(LS, false), (FISTS, true)] {
        let mut w = World::new(sp.clone(), 0.002);
        let a = w.add_fighter("A", attacker, "");
        let b = w.add_fighter("B", FISTS, "");
        w.at(0.1, Input::Attack { who: "A".into(), mv: 0, angle: 0.0 });
        w.run_until(0.104);
        let we = w.cur_m(a).unwrap().attack().unwrap().windup_end;
        w.at(we - 0.1, Input::Parry { who: "B".into(), bt: 0 });
        w.at(we + 0.02, Input::Contact { who: "A".into(), target: "B".into(), bone: "Spine1".into() });
        w.run_until(we + 0.03);
        let parried = w.hits.iter().any(|e| e["kind"] == "parry");
        assert_eq!(parried, blocks, "attacker {attacker}: parried {parried}");
        assert_eq!(w.cur_m(a).unwrap().kind() == "Blocked", blocks);
        let _ = b;
    }
}

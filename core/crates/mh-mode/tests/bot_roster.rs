//! The game's bot roster (ai/bot_profiles.rs) from the packages (extract/json; SKIP without): 35 BOT_* profiles,
//! their behaviour classes and loadouts, BeginPlay's choice on a seeded stream, and a bot added with it.
use mh_mode::ai::bot_profiles::BotRoster;
use mordhau_core::ue::CrtRand;
use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn load(p: &str) -> Option<Vec<Value>> {
    let t = std::fs::read_to_string(root().join("extract/json").join(format!("{p}.json"))).ok()?;
    serde_json::from_str::<Value>(&t).ok()?.as_array().cloned()
}

#[test]
fn roster_from_packages() {
    if load(mh_mode::ai::bot_profiles::SINGLETON).is_none() {
        return eprintln!("SKIP: no extract/json");
    }
    let r = BotRoster::from_exports(&load).unwrap();
    assert_eq!(r.profiles.len(), 35);
    assert_eq!(r.loadouts.len(), 33);
    assert_eq!((r.male_voices, r.female_voices), (27, 9));
    let mut by: BTreeMap<&str, usize> = BTreeMap::new();
    for p in &r.profiles {
        *by.entry(p.behavior.as_str()).or_default() += 1;
    }
    eprintln!("behaviours {by:?}");
    assert_eq!(by.get("BOTBEHAVIOR_Boloncd"), Some(&19));
    assert_eq!(by.get("BOTBEHAVIOR_Worthless"), Some(&11));
    assert!(!by.contains_key("BOTBEHAVIOR_Knight"));
    let duelist = r.profiles.iter().find(|p| p.name == "BOT_Duelist").unwrap();
    assert_eq!(duelist.loadout_id, 14);
    eprintln!("BOT_Duelist weapons {:?}", r.loadouts[14].0);
    assert_eq!(r.loadouts[19].0, vec!["Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword".to_string()]);
    // BeginPlay's draws on a seeded stream: the pick, then 2 voice draws (no BOT_* randomizes appearance)
    let mut rng = CrtRand::new(7);
    let c = r.choose(&mut rng).unwrap();
    eprintln!("seed 7: {} ({}), loadout {:?} {:?}, draws {}", c.name, c.behavior, c.loadout, c.weapons, rng.calls);
    assert_eq!(rng.calls, 3);
    let mut hist: BTreeMap<String, usize> = BTreeMap::new();
    let mut rng = CrtRand::new(1);
    for _ in 0..3500 {
        *hist.entry(r.choose(&mut rng).unwrap().behavior).or_default() += 1;
    }
    eprintln!("3500 picks: {hist:?}");
    assert!(hist["BOTBEHAVIOR_Boloncd"] > 1600);
}

/// a bot added through BeginPlay's roster path gets the chosen class's spec record (or Randomize)
#[test]
fn bot_from_roster_uses_the_chosen_behaviour() {
    let d = root().join("data_gen/spec");
    if load(mh_mode::ai::bot_profiles::SINGLETON).is_none() || !d.join("index.json").exists() {
        return eprintln!("SKIP: no extract/json / spec");
    }
    let spec = mh_spec::Spec::load(&d, false).unwrap();
    let r = BotRoster::from_exports(&load).unwrap();
    let mut w = mh_mode::ai::Bots { k: std::sync::Arc::new(mh_mode::spec_bots::kismet(&spec).unwrap()), ..Default::default() };
    w.add_body(mh_mode::ai::BotBody::new("bot"));
    let preview = w.preview_bot_choice(&r).unwrap();
    struct H;
    impl mh_mode::ai::BotHost for H {
        fn request_attack(&mut self, _b: usize, _m: i64, _a: f64) -> mh_mode::ai::PawnView {
            Default::default()
        }
        fn request_parry(&mut self, _b: usize, _t: i64) -> mh_mode::ai::PawnView {
            Default::default()
        }
        fn request_feint(&mut self, _b: usize) -> mh_mode::ai::PawnView {
            Default::default()
        }
    }
    let profile_of = |n: &str| mh_mode::spec_bots::profile(&spec, n).ok();
    let (_, c) = w.add_bot_from_roster(0, &r, &profile_of, None, 0.0, &mut H).unwrap();
    assert_eq!(c, preview, "the preview is what BeginPlay chose");
    assert_eq!(w.bot(0).profile.p, mh_mode::spec_bots::profile(&spec, &c.behavior).unwrap());
    eprintln!("bot: {} -> {} weapons {:?}", c.name, c.behavior, c.weapons);
}

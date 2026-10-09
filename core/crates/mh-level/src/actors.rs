//! Gameplay actors as data for the sim: every Blueprint actor placed in a map's levels (doors, ladders,
//! destructibles, siege equipment, pickups and spawners, mode objectives, Horde chests / vendors, hazards, ...) with its
//! transform, its instance properties, its class (Blueprint package + Blueprint parent chain + the native class at its
//! root) and the components that carry its collision, so mh-mode / mh-character implement behaviour per class.
//! The class defaults (properties merged over the Blueprint chain, UePkg.defaults) are per class in `classes`.
//!
//! `category` is a coarse grouping for hosts, chosen from the class names of the chain and the native root (rules in
//! `categorize`, ordered; the names are the game's own). The authoritative identity is `class` / `native`.

use crate::level::{class_pkg, LevelData, Pkgs};
use crate::xf::Xf;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::rc::Rc;

#[derive(Clone, Debug)]
pub struct GameplayClass {
    /// Blueprint class package ("Mordhau/Content/.../BP_X")
    pub pkg: String,
    /// Blueprint packages from this class up (child first), as UePkg.chain
    pub chain: Vec<String>,
    /// the native class the chain ends in ("MordhauDestructible", "MordhauEquipment", "Actor", ...)
    pub native: String,
    pub category: &'static str,
    /// class default properties merged over the chain (UePkg.defaults)
    pub defaults: Rc<Map<String, Value>>,
}

#[derive(Clone, Debug)]
pub struct GameplayActor {
    pub level: usize,
    pub name: String,
    /// export Type ("BP_DestroyableWoodenDoor_C")
    pub class: String,
    pub category: &'static str,
    /// root component world transform (UE cm)
    pub xf: Option<Xf>,
    /// the actor's own serialized properties (instance values; class defaults in `LevelData::gameplay_classes`)
    pub props: Map<String, Value>,
    /// the actor export ("<level pkg>.<index>")
    pub path: String,
    /// its static / instanced / spline mesh placements (names as in `LevelData::meshes` / `splines`): the collision
    /// bodies of `collision::CollisionWorld` whose `source` is the same component export
    pub components: Vec<String>,
}

/// Category rules over the chain's class names and the native root, first match wins
pub fn categorize(names: &[String], native: &str) -> &'static str {
    let any = |pats: &[&str]| names.iter().any(|n| pats.iter().any(|p| n.contains(p))) || pats.iter().any(|p| native.contains(p));
    if any(&["NavLink"]) {
        "navigation"
    } else if any(&["PlayerStart", "SpawnPoint", "StartingSpawns"]) {
        "spawn"
    } else if any(&["Ladder"]) {
        "ladder"
    } else if any(&["Door", "Porticulis", "Portcullis", "CastleGate", "Gate_"]) {
        "door"
    } else if any(&["Ballista", "Catapult", "Trebuchet", "Mangonel", "Mortar", "BatteringRam", "SiegeTank", "Siege"]) {
        "siege"
    } else if any(&["Horse"]) {
        "horse"
    } else if any(&["HordeChest", "SellVendor", "BlessingGiver", "TreasureLoot", "SuperWeaponChest", "GoldChest", "GoldPile", "DungeonChest"]) {
        "horde_shop"
    } else if any(&["Horde", "Demon", "BossManager"]) {
        "horde_objective"
    } else if any(&["Frontline", "FLFortification", "Push", "Capture", "Deliver", "KillObjective", "ProgressDriver", "ProgressSlave",
        "ProgressActor", "Crank", "Lever", "ObjectiveTracker", "TeamBase", "Killable", "Burnable", "DummyObjective"])
    {
        "objective"
    } else if any(&["Destroyable", "Destructible", "Breakable", "TreeBreaker", "Barricade"]) {
        "destructible"
    } else if any(&["EquipmentSpawner", "RandomEquipment", "AmmoBox", "WeaponBundle", "MysteryWeapon", "Consumable", "Medpack", "Bandage",
        "Equipment", "Weapon", "Spawner"])
    {
        "pickup"
    } else if any(&["Impalement", "FireField", "FlameBarrier", "OutOfBounds", "OOBBarrier", "DeathBarrier", "ForceFall", "OilCauldron",
        "IcicleTrap", "Spike", "Lightning", "SwingingLog", "LedgePusher", "ForceRagdoll", "Fire", "Explosive"])
    {
        "hazard"
    } else if any(&["SpawnProtection", "ToolboxPrevention", "ContainmentBox", "Volume", "Box"]) {
        "volume"
    } else if any(&["Hittable", "Chandelier", "HangingCage", "HangingChain", "Movable", "Bell", "Physics"]) {
        "physics_prop"
    } else if any(&["Particle", "Sound", "Audio", "Music", "Sky", "Cloud", "Storm", "Decal", "SceneEffects", "Crowd", "Camera", "Ambient"]) {
        "presentation"
    } else {
        "other"
    }
}

/// Collect every Blueprint actor of the read map (exports whose Outer is a level and whose Type is a Blueprint class)
pub fn read(pk: &Pkgs, d: &mut LevelData) {
    let mut classes: BTreeMap<String, GameplayClass> = BTreeMap::new();
    let mut by_actor: BTreeMap<(usize, String), Vec<String>> = BTreeMap::new();
    for m in &d.meshes {
        by_actor.entry((m.level, m.actor.clone())).or_default().push(m.name.clone());
    }
    for s in &d.splines {
        by_actor.entry((s.level, s.actor.clone())).or_default().push(s.name.clone());
    }
    let mut out = vec![];
    for (li, lv) in d.levels.iter().enumerate() {
        let exps = pk.load_pkg(&lv.pkg);
        let Some(level_idx) = exps.iter().position(|e| e.get("Type").and_then(|t| t.as_str()) == Some("Level")) else { continue };
        let suffix = format!(".{level_idx}");
        for (ei, e) in exps.iter().enumerate() {
            let t = e.get("Type").and_then(|v| v.as_str()).unwrap_or("");
            if !t.ends_with("_C") || ei == level_idx {
                continue;
            }
            let outer = e.get("Outer").and_then(|o| o.get("ObjectPath")).and_then(|v| v.as_str()).unwrap_or("");
            if !outer.ends_with(&suffix) {
                continue;
            }
            if !classes.contains_key(t) {
                let cp = class_pkg(e);
                let chain = pk.rd.chain(&cp);
                let native = chain
                    .last()
                    .map(|last| pk.rd.super_of(last))
                    .and_then(|s| s.get("ObjectName").and_then(|v| v.as_str()).map(|n| n.split('\'').nth(1).unwrap_or(n).to_string()))
                    .unwrap_or_default();
                let names: Vec<String> = chain.iter().map(|c| c.rsplit('/').next().unwrap_or(c).to_string()).collect();
                let category = categorize(&names, &native);
                classes.insert(t.to_string(), GameplayClass { pkg: cp.clone(), chain, native, category, defaults: pk.defaults(&cp) });
            }
            let cat = classes[t].category;
            let p = pk.props(e);
            let rc = pk.obj(p.get("RootComponent"));
            let name = e.get("Name").and_then(|v| v.as_str()).unwrap_or("").to_string();
            out.push(GameplayActor {
                level: li,
                components: by_actor.get(&(li, name.clone())).cloned().unwrap_or_default(),
                name,
                class: t.to_string(),
                category: cat,
                xf: rc.map(|rc| lv.xf * pk.world_xf(&rc)),
                props: p,
                path: format!("{}.{ei}", lv.pkg),
            });
        }
    }
    d.gameplay_classes = classes;
    d.gameplay = out;
}

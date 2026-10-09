//! A BehaviorTree package as a `TreeDef` (rust-mode-ai r5): the Rust port of godot/components/ue/records/bot_data.gd
//! `tree_def` (`_node`, `_aux`, `_task_params`, `_class_names`, `_with_native`), so the exe-mode data path can read any
//! tree from the paks (BT_Horde, BT_CombatHorde, ...) without the Godot exporter. No I/O: `load` hands the package's
//! export list (mh-pak `export_json` / extract/json shape: {"Type", "Name", "Properties"}, object references
//! {"ObjectName", "ObjectPath": "<package>.<export index>"}).
//!
//! Native Mordhau task defaults under the node's serialized properties (bot_data.gd `_with_native` -> NativeCtor):
//!   UBTTask_SwitchEquipment ctor rva=0x144dcb0: bMelee true, AllowedSubclasses / NotAllowedSubclasses empty
//!   UBTTask_VoiceOrEmote ctor rva=0x144dd00: Chance 0.1, GlobalCooldown 2.0, bForceEmote false, lists empty
//! Engine defaults (bot_data.gd, UE 4.26 source, UNCONFIRMED against the exe): BTTask_Wait WaitTime 5 / RandomDeviation
//! 0, BTDecorator_Cooldown CoolDownTime 5, UBTService Interval 0.5 / RandomDeviation 0.1 (UBTService::UBTService
//! rva=0x3895d00, CONFIRMED in bt_tree.gd R7), FlowAbortMode None, bInverseCondition false.

use super::bt::{AuxDef, ChildDef, NodeDef, TaskParams, TreeDef};
use serde_json::Value;

const BP_TASKS: &str = "Mordhau/Content/Mordhau/AI/Tasks/";

pub type Loader<'a> = &'a dyn Fn(&str) -> Option<Vec<Value>>;

fn pkg_of(obj_path: &str) -> &str {
    match obj_path.rfind('.') {
        Some(i) if obj_path[i + 1..].chars().all(|c| c.is_ascii_digit()) => &obj_path[..i],
        _ => obj_path,
    }
}

fn ref_index(v: &Value) -> Option<usize> {
    let p = v.get("ObjectPath")?.as_str()?;
    p.rsplit('.').next()?.parse().ok()
}

fn ref_pkg(v: &Value) -> Option<String> {
    v.get("ObjectPath").and_then(|p| p.as_str()).map(|p| pkg_of(p).to_string())
}

fn props(e: &Value) -> &serde_json::Map<String, Value> {
    static EMPTY: std::sync::OnceLock<serde_json::Map<String, Value>> = std::sync::OnceLock::new();
    e.get("Properties").and_then(|p| p.as_object()).unwrap_or_else(|| EMPTY.get_or_init(Default::default))
}

/// a float property as the package JSON holds it; `d` when absent (a native ctor default is its f32 widened, an
/// engine default the reader's literal, as bot_data.gd does)
fn f(p: &serde_json::Map<String, Value>, k: &str, d: f64) -> f64 {
    p.get(k).and_then(|v| v.as_f64()).unwrap_or(d)
}

fn b(p: &serde_json::Map<String, Value>, k: &str, d: bool) -> bool {
    p.get(k).and_then(|v| v.as_bool()).unwrap_or(d)
}

fn key(p: &serde_json::Map<String, Value>, k: &str) -> Option<String> {
    p.get(k)?.get("SelectedKeyName")?.as_str().map(String::from)
}

/// "EBTFlowAbortMode::LowerPriority" -> "LowerPriority"
fn enum_tail(v: Option<&Value>, d: &str) -> String {
    v.and_then(|x| x.as_str()).map(|s| s.rsplit("::").next().unwrap_or(s).to_string()).unwrap_or_else(|| d.to_string())
}

/// class references "BlueprintGeneratedClass'BP_X_C'" -> "BP_X" (bot_data.gd `_class_names`)
fn class_names(p: &serde_json::Map<String, Value>, k: &str) -> Vec<String> {
    p.get(k)
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.get("ObjectName").and_then(|o| o.as_str()))
                .map(|o| o.split('\'').nth(1).unwrap_or(o).trim_end_matches("_C").to_string())
                .collect()
        })
        .unwrap_or_default()
}

fn cdo(load: Loader, pkg: &str) -> Option<serde_json::Map<String, Value>> {
    let ex = load(pkg)?;
    ex.iter().find(|e| e.get("Name").and_then(|n| n.as_str()).map(|n| n.starts_with("Default__")).unwrap_or(false)).map(|e| props(e).clone())
}

/// read a BehaviorTree package (`path` without extension) and its RunBehavior subtrees
pub fn tree_def(load: Loader, path: &str) -> Result<TreeDef, String> {
    let ex = load(path).ok_or_else(|| format!("{path}: package not found"))?;
    let bt = ex.iter().find(|e| e.get("Type").and_then(|t| t.as_str()) == Some("BehaviorTree")).ok_or_else(|| format!("{path}: no BehaviorTree export"))?;
    let p = props(bt);
    let mut out = TreeDef { asset: path.to_string(), ..Default::default() };
    if let Some(bb) = p.get("BlackboardAsset").and_then(ref_pkg) {
        let bex = load(&bb).ok_or_else(|| format!("{bb}: blackboard not found"))?;
        if let Some(d) = bex.iter().find(|e| e.get("Type").and_then(|t| t.as_str()) == Some("BlackboardData")) {
            for k in props(d).get("Keys").and_then(|v| v.as_array()).into_iter().flatten() {
                let name = k.get("EntryName").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let cls = k.get("KeyType").and_then(|t| t.get("ObjectName")).and_then(|o| o.as_str()).unwrap_or("");
                let cls = cls.split('\'').next().unwrap_or("").trim_start_matches("BlackboardKeyType_").to_string();
                out.blackboard.insert(name, cls);
            }
        }
    }
    let root = p.get("RootNode").and_then(ref_index).ok_or_else(|| format!("{path}: no RootNode"))?;
    let mut keys = std::mem::take(&mut out.blackboard);
    out.root = node(load, &ex, root, &mut keys, path)?;
    out.blackboard = keys;
    Ok(out)
}

fn node(load: Loader, ex: &[Value], i: usize, keys: &mut std::collections::BTreeMap<String, String>, path: &str) -> Result<NodeDef, String> {
    let e = ex.get(i).ok_or_else(|| format!("{path}: node index {i} out of range"))?;
    let mut n = NodeDef {
        name: e.get("Name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        type_: e.get("Type").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        ..Default::default()
    };
    let p = props(e);
    match n.type_.as_str() {
        "BTComposite_Selector" => n.kind = "selector".into(),
        "BTComposite_Sequence" => n.kind = "sequence".into(),
        "BTTask_RunBehavior" => {
            n.kind = "subtree".into();
            let ba = p.get("BehaviorAsset").ok_or_else(|| format!("{path}.{}: no BehaviorAsset", n.name))?;
            n.subtree_name = ba.get("ObjectName").and_then(|o| o.as_str()).unwrap_or("").to_string();
            let sub = tree_def(load, &ref_pkg(ba).unwrap_or_default())?;
            for (k, v) in sub.blackboard {
                keys.insert(k, v);
            }
            n.subtree = Some(Box::new(sub.root));
        }
        _ => {
            n.kind = "task".into();
            n.params = task_params(load, &n.type_, p);
        }
    }
    for s in p.get("Services").and_then(|v| v.as_array()).into_iter().flatten() {
        if let Some(si) = ref_index(s) {
            n.services.push(aux(ex, si, path)?);
        }
    }
    for ch in p.get("Children").and_then(|v| v.as_array()).into_iter().flatten() {
        let r = match ch.get("ChildComposite") {
            Some(c) if !c.is_null() => c,
            _ => ch.get("ChildTask").ok_or_else(|| format!("{path}.{}: child without a node", n.name))?,
        };
        let ci = ref_index(r).ok_or_else(|| format!("{path}.{}: bad child reference", n.name))?;
        let mut c = ChildDef { node: node(load, ex, ci, keys, path)?, decorators: vec![] };
        for d in ch.get("Decorators").and_then(|v| v.as_array()).into_iter().flatten() {
            if let Some(di) = ref_index(d) {
                c.decorators.push(aux(ex, di, path)?);
            }
        }
        n.children.push(c);
    }
    Ok(n)
}

fn task_params(load: Loader, ty: &str, p: &serde_json::Map<String, Value>) -> TaskParams {
    let mut t = TaskParams::default();
    match ty {
        "BTTask_Wait" => {
            t.wait_time = f(p, "WaitTime", 5.0);
            t.random_deviation = f(p, "RandomDeviation", 0.0);
        }
        "BTTask_SwitchEquipment" => {
            t.b_melee = b(p, "bMelee", true);
            t.allowed_subclasses = class_names(p, "AllowedSubclasses");
            t.not_allowed_subclasses = class_names(p, "NotAllowedSubclasses");
        }
        "BTTask_VoiceOrEmote" => {
            t.chance = f(p, "Chance", 0.1f32 as f64);
            t.global_cooldown = f(p, "GlobalCooldown", 2.0);
            t.b_force_emote = b(p, "bForceEmote", false);
            t.voice_commands = ints(p.get("VoiceCommandsList"));
            t.emotes = ints(p.get("EmotesList"));
        }
        "BTTask_FindRandomLocation_C" | "BTTask_FindUnstuckSpot_C" | "BTTask_MoveToDestination_C" | "BTTask_FindHordeTask_C" => {
            // Blueprint tasks: the node's selectors / variables over the class CDO's (AI/Tasks/<class> for the shared
            // ones; the horde tasks keep their selectors on the node)
            let c = cdo(load, &format!("{BP_TASKS}{}", ty.trim_end_matches("_C"))).unwrap_or_default();
            let get = |k: &str| p.get(k).or_else(|| c.get(k));
            t.target_location_key = get("TargetLocation").and_then(|v| v.get("SelectedKeyName")).and_then(|v| v.as_str()).unwrap_or("").to_string();
            if ty == "BTTask_MoveToDestination_C" {
                t.target_actor_key = get("TargetActor").and_then(|v| v.get("SelectedKeyName")).and_then(|v| v.as_str()).unwrap_or("").to_string();
                t.acceptable_radius = get("AcceptableRadius").and_then(|v| v.as_f64()).unwrap_or(0.0);
                t.use_midpoint = get("UseMidpoint").and_then(|v| v.as_bool()).unwrap_or(false);
                t.force_walk = get("ForceWalk").and_then(|v| v.as_bool()).unwrap_or(false);
            }
        }
        _ => {}
    }
    t
}

fn ints(v: Option<&Value>) -> Vec<i64> {
    v.and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_i64().or_else(|| x.as_str().and_then(|s| s.rsplit("::").next()?.parse().ok())))
                .collect()
        })
        .unwrap_or_default()
}

fn aux(ex: &[Value], i: usize, path: &str) -> Result<AuxDef, String> {
    let e = ex.get(i).ok_or_else(|| format!("{path}: aux node index {i} out of range"))?;
    let p = props(e);
    let mut a = AuxDef {
        name: e.get("Name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        type_: e.get("Type").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        flow_abort_mode: enum_tail(p.get("FlowAbortMode"), "None"),
        b_inverse_condition: b(p, "bInverseCondition", false),
        ..Default::default()
    };
    match a.type_.as_str() {
        "BTDecorator_Cooldown" => a.cooldown_time = f(p, "CoolDownTime", 5.0),
        "BTDecorator_Blackboard" => {
            a.blackboard_key = key(p, "BlackboardKey").unwrap_or_default();
            a.operation_type = p.get("OperationType").and_then(|v| v.as_i64()).unwrap_or(0);
            a.float_value = f(p, "FloatValue", 0.0);
        }
        "BTService_DMPerceptionUpdate_C" | "BTService_HordePerceptionUpdate_C" => {
            a.perceives_enemy_key = key(p, "bPerceivesEnemy").unwrap_or_default();
            a.closest_enemy_distance_key = key(p, "ClosestEnemyDistance").unwrap_or_default();
        }
        _ => {}
    }
    if a.type_.starts_with("BTService") {
        a.interval = f(p, "Interval", 0.5);
        a.random_deviation = f(p, "RandomDeviation", 0.1);
    }
    Ok(a)
}

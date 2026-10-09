//! A behavior-tree runner for the game's own trees (godot/game/ai/bt/bt_tree.gd: BT_Deathmatch -> BT_CombatGeneric),
//! built from typed node records (the reference's BotData.tree_def, read from the BehaviorTree packages: composites,
//! decorators, services, tasks and their parameters). The host reads the package; TreeDef is a serde record.
//!
//! UE's BehaviorTree runtime is engine code; only what these trees use is ported. Rules (as bt_tree.gd):
//!   R1 Selector: next child after a failure, done on success / Sequence: next after a success, done on failure
//!      (UE 4.26 docs "Behavior Tree Node Reference: Composites"; UBTComposite_Selector::GetNextChildHandler
//!      rva=0x38ab4f0 not disassembled).
//!   R2 A child whose decorators fail counts as Failed; the child's decorators are then told the node was processed
//!      (OnNodeProcessed), which is where ForceSuccess turns it into Succeeded. CONFIRMED: UBTCompositeNode::
//!      FindChildToExecute rva=0x387c820 (disasm 0x14387c8c3); UBTDecorator_ForceSuccess::OnNodeProcessed rva=0x1b6e110.
//!   R3 Cooldown passes when now - LastUseTimestamp >= CoolDownTime (UBTDecorator_Cooldown::CalculateRawConditionValue
//!      rva=0x389c0c0, `comiss; setae`); LastUseTimestamp starts at -FLT_MAX (InitializeMemory rva=0x38af6b0) and is
//!      set to now when the decorated node deactivates (OnNodeDeactivation rva=0x38b2b20). CONFIRMED.
//!   R4 Blackboard decorator: bool key "Is Set" (0) / "Is Not Set" (1); float key arithmetic Equal 0, NotEqual 1,
//!      Less 2, LessOrEqual 3, Greater 4, GreaterOrEqual 5 (UE 4.26 EArithmeticKeyOperation; UNCONFIRMED: engine enum).
//!   R5 One execution request per BT tick: a task that finishes at once calls OnTaskFinished -> RequestExecution, which
//!      ends in ScheduleNextTick (CONFIRMED: disasm of OnTaskFinished rva=0x3885cd0 and RequestExecution rva=0x388a2c0).
//!   R6 Tick order inside UBehaviorTreeComponent::TickComponent rva=0x388e990: auxiliary-node tick (identity of the
//!      first call UNCONFIRMED), ProcessExecutionRequest, parallel tasks, then WrappedTickTask on the active task.
//!   R7 Services: Interval 0.5, RandomDeviation 0.1 (UBTService::UBTService rva=0x3895d00), next tick after
//!      max(0, I - D) + FRand * ((I + D) - max(0, I - D)) (UBTService::ScheduleNextTick rva=0x38b8190, one rand()).
//!      UNCONFIRMED: a service ticks once when its composite becomes active.
//!   R8 Observer aborts: a LowerPriority decorator whose condition becomes true while a lower-priority sibling branch
//!      runs aborts that branch and re-runs its parent from the decorated child (UNCONFIRMED: re-evaluated each tick).
//!   R9 When the root composite finishes the tree starts again from the root, at most once per tick. UNCONFIRMED.
//!   R10 Wait: remaining = max(0, W - D) + FRand * ((W + D) - max(0, W - D)) (UBTTask_Wait::ExecuteTask rva=0x38a5d40),
//!      each tick remaining -= dt, Succeeded once remaining <= 0 (TickTask rva=0x38bb8c0). CONFIRMED.
//!   R11 RunBehavior runs the referenced tree in place; its result is the subtree root's result. UE docs.
//! BTService_DMPerceptionUpdate_C (bytecode ExecuteUbergraph_BTService_DMPerceptionUpdate [132..455]): GetClosestEnemy
//! valid -> bPerceivesEnemy = true and ClosestEnemyDistance = VSize(enemy - pawn); else ClearBlackboardValue
//! (bPerceivesEnemy) (a cleared bool reads false).

use super::controller::{BotController, Ctx};
use super::tasks::{Task, TaskNode};
use crate::consts::bot as K;
use mordhau_core::ue::{maxf, FVector};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const SUCCEEDED: i32 = 0;
pub const FAILED: i32 = 1;
pub const ABORTED: i32 = 2;
pub const IN_PROGRESS: i32 = 3;
pub const RESULT_NAMES: [&str; 4] = ["Succeeded", "Failed", "Aborted", "InProgress"];

// ---- records (BotData.BtAux / BtTaskParams / BtNodeDef / BtChildDef / TreeDef) ------------------------------------
/// a decorator or service
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AuxDef {
    pub name: String,
    #[serde(rename = "type")]
    pub type_: String,
    /// UBTDecorator FlowAbortMode (EBTFlowAbortMode, ctor None)
    pub flow_abort_mode: String,
    pub b_inverse_condition: bool,
    /// BTDecorator_Cooldown CoolDownTime (engine ctor 5.0, UNCONFIRMED)
    pub cooldown_time: f64,
    pub blackboard_key: String,
    pub operation_type: i64,
    pub float_value: f64,
    /// UBTService Interval / RandomDeviation (UBTService::UBTService rva=0x3895d00: 0.5 / 0.1)
    pub interval: f64,
    pub random_deviation: f64,
    pub perceives_enemy_key: String,
    pub closest_enemy_distance_key: String,
}

/// a task's parameters (by task class)
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskParams {
    /// BTTask_Wait WaitTime / RandomDeviation (engine ctor 5.0 / 0, UNCONFIRMED)
    pub wait_time: f64,
    pub random_deviation: f64,
    /// UBTTask_SwitchEquipment +0x70 / +0x78 / +0x88 (ctor rva=0x144dcb0)
    pub b_melee: bool,
    pub allowed_subclasses: Vec<String>,
    pub not_allowed_subclasses: Vec<String>,
    /// UBTTask_VoiceOrEmote +0x98 / +0x94 / +0x70 / +0x80 / +0x90 (ctor rva=0x144dd00)
    pub chance: f64,
    pub global_cooldown: f64,
    pub voice_commands: Vec<i64>,
    pub emotes: Vec<i64>,
    pub b_force_emote: bool,
    /// Blueprint tasks (AI/Tasks/BTTask_*_C): blackboard selectors / CDO variables
    pub target_location_key: String,
    pub target_actor_key: String,
    pub acceptable_radius: f64,
    pub use_midpoint: bool,
    pub force_walk: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NodeDef {
    pub name: String,
    #[serde(rename = "type")]
    pub type_: String,
    /// "selector" | "sequence" | "task" | "subtree"
    pub kind: String,
    pub children: Vec<ChildDef>,
    pub services: Vec<AuxDef>,
    pub subtree: Option<Box<NodeDef>>,
    pub subtree_name: String,
    pub params: TaskParams,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChildDef {
    pub node: NodeDef,
    pub decorators: Vec<AuxDef>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TreeDef {
    pub asset: String,
    /// blackboard key -> "Bool" | "Float" | "Object" | "Vector" ...
    pub blackboard: BTreeMap<String, String>,
    pub root: NodeDef,
}

impl TreeDef {
    pub fn from_json(text: &str) -> Result<TreeDef, String> {
        serde_json::from_str(text).map_err(|e| format!("TreeDef: {e}"))
    }
}

/// a blackboard value
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BbVal {
    Bool(bool),
    Float(f64),
    Body(usize),
    Vec(FVector),
}

impl BbVal {
    /// GDScript bool(v)
    pub fn truthy(&self) -> bool {
        match *self {
            BbVal::Bool(b) => b,
            BbVal::Float(f) => f != 0.0,
            BbVal::Body(_) => true,
            BbVal::Vec(v) => v != FVector::ZERO,
        }
    }
    /// GDScript float(v)
    pub fn as_f(&self) -> f64 {
        match *self {
            BbVal::Bool(b) => b as i64 as f64,
            BbVal::Float(f) => f,
            _ => 0.0,
        }
    }
}

// ---- runtime ------------------------------------------------------------------------------------------------------
#[derive(Clone, Debug)]
pub struct Decorator {
    pub def: AuxDef,
    /// Cooldown memory (R3): -FLT_MAX
    pub last_use: f64,
}

#[derive(Clone, Debug)]
pub struct Service {
    pub def: AuxDef,
    pub next_time: f64,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub name: String,
    pub type_: String,
    pub kind: String,
    pub subtree_name: String,
    pub children: Vec<usize>,
    pub child_decorators: Vec<Vec<Decorator>>,
    pub services: Vec<Service>,
    pub task: Option<TaskNode>,
    /// RunBehavior: the referenced tree's root composite
    pub subtree: Option<usize>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    pub node: usize,
    pub idx: i64,
    pub via: Option<usize>,
}

#[derive(Clone, Debug)]
pub struct BtTree {
    pub asset: String,
    pub nodes: Vec<Node>,
    pub root: usize,
    pub bb_types: BTreeMap<String, String>,
    pub stack: Vec<Frame>,
    /// the latent task's node
    pub active: Option<usize>,
    pub pending: bool,
    pub pending_result: i32,
    pub pending_child: Option<usize>,
    restarted_tick: i64,
    tick_n: i64,
}

impl BtTree {
    pub fn new(def: &TreeDef) -> BtTree {
        let mut t = BtTree {
            asset: def.asset.clone(),
            nodes: Vec::new(),
            root: 0,
            bb_types: def.blackboard.clone(),
            stack: Vec::new(),
            active: None,
            pending: true,
            pending_result: -1,
            pending_child: None,
            restarted_tick: -1,
            tick_n: 0,
        };
        t.root = t.build(&def.root);
        t
    }

    fn build(&mut self, d: &NodeDef) -> usize {
        let i = self.nodes.len();
        self.nodes.push(Node {
            name: d.name.clone(),
            type_: d.type_.clone(),
            kind: d.kind.clone(),
            subtree_name: d.subtree_name.clone(),
            children: Vec::new(),
            child_decorators: Vec::new(),
            services: d.services.iter().map(|s| Service { def: s.clone(), next_time: 0.0 }).collect(),
            task: None,
            subtree: None,
        });
        if d.kind == "subtree" {
            if let Some(st) = &d.subtree {
                let s = self.build(st);
                self.nodes[i].subtree = Some(s);
            }
        } else if d.kind == "task" {
            self.nodes[i].task = Some(TaskNode::new(&d.name, &d.type_, d.params.clone()));
        }
        for ch in &d.children {
            let c = self.build(&ch.node);
            self.nodes[i].children.push(c);
            self.nodes[i]
                .child_decorators
                .push(ch.decorators.iter().map(|de| Decorator { def: de.clone(), last_use: -3.4028234663852886e38 }).collect());
        }
        i
    }

    /// the active task node's name ("" = none)
    pub fn active_name(&self) -> &str {
        self.active.map(|a| self.nodes[a].name.as_str()).unwrap_or("")
    }

    pub fn tick(&mut self, c: &mut BotController, cx: &mut Ctx, dt: f64) {
        self.tick_n += 1;
        self.tick_services(c, cx); // R6: auxiliary nodes first
        self.check_aborts(c, cx); // R8
        if self.pending {
            self.search(c, cx); // R5
        }
        if let Some(a) = self.active {
            // R6: the active task, also one that just went latent
            let r = self.nodes[a].task.as_mut().unwrap().tick(c, cx, dt);
            if r != IN_PROGRESS {
                self.finish(c, cx, a, r);
            }
        }
    }

    fn finish(&mut self, c: &mut BotController, cx: &Ctx, n: usize, r: i32) {
        let s = format!("{} finished {}", self.nodes[n].name, RESULT_NAMES[r as usize]);
        c.note(cx, "BT", s, "");
        self.active = None;
        self.pending = true; // OnTaskFinished -> RequestExecution -> next tick (R5)
        self.pending_result = r;
        self.pending_child = Some(n);
    }

    fn decorators_of(&self, fr: Frame, idx: i64) -> Option<(usize, usize)> {
        let n = &self.nodes[fr.node];
        if idx >= 0 && (idx as usize) < n.child_decorators.len() {
            Some((fr.node, idx as usize))
        } else {
            None
        }
    }

    /// the decorated child deactivates: OnNodeProcessed (ForceSuccess) and OnNodeDeactivation (Cooldown stamp)
    fn deactivate(&mut self, now: f64, at: Option<(usize, usize)>, r: i32) -> i32 {
        let mut r = r;
        if let Some((n, i)) = at {
            for d in self.nodes[n].child_decorators[i].iter_mut() {
                if d.def.type_ == "BTDecorator_ForceSuccess" {
                    r = SUCCEEDED;
                } else if d.def.type_ == "BTDecorator_Cooldown" {
                    d.last_use = now;
                }
            }
        }
        r
    }

    fn push(&mut self, c: &mut BotController, cx: &mut Ctx, n: usize, via: Option<usize>) {
        self.stack.push(Frame { node: n, idx: -1, via });
        for s in 0..self.nodes[n].services.len() {
            self.service_tick(c, cx, n, s); // R7 first tick on activation (UNCONFIRMED)
        }
    }

    fn search(&mut self, c: &mut BotController, cx: &mut Ctx) {
        self.pending = false;
        let mut res = self.pending_result;
        if self.pending_child.is_some() && !self.stack.is_empty() {
            let b = *self.stack.last().unwrap();
            res = self.deactivate(cx.now, self.decorators_of(b, b.idx), res);
        }
        self.pending_child = None;
        for _guard in 0..1000 {
            if self.stack.is_empty() {
                if self.restarted_tick == self.tick_n {
                    // R9: once per tick
                    self.pending = true;
                    self.pending_result = -1;
                    return;
                }
                self.restarted_tick = self.tick_n;
                let s = format!("start {}", self.asset.rsplit('/').next().unwrap_or(""));
                c.note(cx, "BT", s, "");
                let root = self.root;
                self.push(c, cx, root, None);
                res = -1;
                continue;
            }
            let fr = *self.stack.last().unwrap();
            let n = fr.node;
            let len = self.nodes[n].children.len() as i64;
            let nxt: i64;
            if fr.idx < 0 {
                nxt = if len > 0 { 0 } else { -1 };
                if len == 0 {
                    res = FAILED; // empty composite (UNCONFIRMED)
                }
            } else if self.nodes[n].kind == "selector" {
                nxt = if res != SUCCEEDED && fr.idx + 1 < len { fr.idx + 1 } else { -1 }; // R1
            } else {
                nxt = if res == SUCCEEDED && fr.idx + 1 < len { fr.idx + 1 } else { -1 }; // R1
            }
            if nxt < 0 {
                self.stack.pop();
                if let Some(&b) = self.stack.last() {
                    res = self.deactivate(cx.now, self.decorators_of(b, b.idx), res);
                }
                continue;
            }
            self.stack.last_mut().unwrap().idx = nxt;
            let ch = self.nodes[n].children[nxt as usize];
            let at = self.decorators_of(*self.stack.last().unwrap(), nxt);
            if !self.allowed(c, cx, at) {
                res = FAILED; // R2
                if let Some((dn, di)) = at {
                    if self.nodes[dn].child_decorators[di].iter().any(|d| d.def.type_ == "BTDecorator_ForceSuccess") {
                        res = SUCCEEDED;
                    }
                }
                continue;
            }
            let kind = self.nodes[ch].kind.clone();
            match kind.as_str() {
                "selector" | "sequence" => {
                    self.push(c, cx, ch, None);
                    res = -1;
                }
                "subtree" => {
                    let s = format!("{} -> {}", self.nodes[ch].name, self.nodes[ch].subtree_name);
                    c.note(cx, "BT", s, "");
                    if let Some(st) = self.nodes[ch].subtree {
                        self.push(c, cx, st, Some(ch));
                    }
                    res = -1;
                }
                _ => {
                    let r = self.nodes[ch].task.as_mut().unwrap().execute(c, cx);
                    let s = format!("execute {} -> {}", self.nodes[ch].name, RESULT_NAMES[r as usize]);
                    c.note(cx, "BT", s, "");
                    if r == IN_PROGRESS {
                        self.active = Some(ch);
                        return;
                    }
                    self.pending = true; // R5
                    self.pending_result = r;
                    self.pending_child = Some(ch);
                    return;
                }
            }
        }
        c.note(cx, "BT", "search did not settle (1000 steps)".into(), "");
    }

    fn allowed(&self, c: &mut BotController, cx: &mut Ctx, at: Option<(usize, usize)>) -> bool {
        let Some((n, i)) = at else { return true };
        for d in &self.nodes[n].child_decorators[i] {
            if !self.condition(c, cx, d) {
                return false;
            }
        }
        true
    }

    fn condition(&self, c: &mut BotController, cx: &mut Ctx, d: &Decorator) -> bool {
        let mut ok = true;
        match d.def.type_.as_str() {
            "BTDecorator_Cooldown" => ok = cx.now - d.last_use >= d.def.cooldown_time, // R3
            "BTDecorator_Blackboard" => {
                let key = &d.def.blackboard_key;
                let op = d.def.operation_type;
                let val = c.blackboard.get(key).copied();
                match self.bb_types.get(key).map(|s| s.as_str()).unwrap_or("") {
                    "Bool" => {
                        let b = val.map(|v| v.truthy()).unwrap_or(false);
                        ok = if op == 0 { b } else { !b }; // R4
                    }
                    "Float" => {
                        let x = val.map(|v| v.as_f()).unwrap_or(0.0);
                        let y = d.def.float_value;
                        ok = match op {
                            0 => x == y,
                            1 => x != y,
                            2 => x < y,
                            3 => x <= y,
                            4 => x > y,
                            5 => x >= y,
                            _ => false,
                        };
                    }
                    _ => ok = if op == 0 { val.is_some() } else { val.is_none() },
                }
                if d.def.b_inverse_condition {
                    ok = !ok;
                }
            }
            "BTDecorator_ForceSuccess" => ok = true,
            t => {
                let s = format!("decorator {t} not ported (passes)");
                c.note(cx, "BT", s, "");
            }
        }
        ok
    }

    /// R8: a LowerPriority observer on an earlier child of a running composite whose condition now holds
    fn check_aborts(&mut self, c: &mut BotController, cx: &mut Ctx) {
        for si in 0..self.stack.len() {
            let fr = self.stack[si];
            for i in 0..fr.idx.max(0) {
                let at = self.decorators_of(fr, i);
                let observing = at
                    .map(|(n, di)| {
                        self.nodes[n].child_decorators[di]
                            .iter()
                            .any(|d| d.def.flow_abort_mode == "LowerPriority" || d.def.flow_abort_mode == "Both")
                    })
                    .unwrap_or(false);
                if observing && self.allowed(c, cx, at) {
                    let s = format!("abort lower priority: {}", self.nodes[self.nodes[fr.node].children[i as usize]].name);
                    c.note(cx, "BT", s, "");
                    if let Some(a) = self.active.take() {
                        self.nodes[a].task.as_mut().unwrap().abort(c, cx);
                    }
                    self.stack.truncate(si + 1);
                    self.stack[si].idx = i - 1;
                    self.pending = true;
                    self.pending_result = FAILED;
                    self.pending_child = None;
                    return;
                }
            }
        }
    }

    fn tick_services(&mut self, c: &mut BotController, cx: &mut Ctx) {
        for fi in 0..self.stack.len() {
            let n = self.stack[fi].node;
            for s in 0..self.nodes[n].services.len() {
                if cx.now >= self.nodes[n].services[s].next_time {
                    self.service_tick(c, cx, n, s);
                }
            }
        }
    }

    fn service_tick(&mut self, c: &mut BotController, cx: &mut Ctx, n: usize, s: usize) {
        let def = self.nodes[n].services[s].def.clone();
        if def.type_ == "BTService_DMPerceptionUpdate_C" {
            // BTService_DMPerceptionUpdate_C bytecode (header)
            let en = c.closest_enemy(cx);
            c.blackboard.insert(def.perceives_enemy_key.clone(), BbVal::Bool(en.is_some()));
            if let Some(en) = en {
                let d = (cx.bodies[en].location - cx.bodies[c.body].location).length() as f64;
                c.blackboard.insert(def.closest_enemy_distance_key.clone(), BbVal::Float(d));
            }
        }
        if def.type_ == "BTService_HordePerceptionUpdate_C" {
            super::tasks::horde_perception_update(c, cx, &def.perceives_enemy_key, &def.closest_enemy_distance_key);
        }
        // else: BTService_ObstacleNavigator_C (navigation, Blueprint without bytecode): not ported
        // R7 UBTService::ScheduleNextTick
        let (iv, dv) = (def.interval, def.random_deviation);
        let lo = maxf(iv - dv, 0.0);
        let r = cx.rng.rand() as f64;
        let q = cx.q;
        self.nodes[n].services[s].next_time = q(q(cx.now + q(q(r * q(q(iv + dv) - lo)) * K::BT_SERVICE_RAND_SCALE)) + lo);
    }

    /// the latent / finished task states (golden traces)
    pub fn task_state(&self, n: usize) -> Option<&Task> {
        self.nodes[n].task.as_ref().map(|t| &t.kind)
    }
}

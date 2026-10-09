//! A Blueprint (Kismet) virtual machine for the game's UMG widgets and HUD: it loads Blueprint generated classes from
//! the paks (functions decoded by kismet.rs, members, class defaults, the WidgetTree), instantiates widgets the way
//! UUserWidget::Initialize does (the class's WidgetTree duplicated per instance, widget variables bound by name,
//! ComponentDelegateBindings bound) and runs their bytecode the way UObject::ProcessEvent / FFrame::Step do (UE 4.26
//! ScriptCore.cpp semantics: EX_Context switches the object the inner expression reads instance variables from, call
//! parameters are evaluated in the caller's frame; the ubergraph's locals persist on the object; latent actions resume
//! the ubergraph at their Linkage).
//!
//! Native functions (UMG widget methods, Kismet libraries, Mordhau's C++ API) are natives.rs; game state the widgets read
//! comes from proxy objects the host fills (host.rs).

use crate::kismet::{self, Ex, Func, ObjRef, CPF_OUT_PARM, CPF_PARM, CPF_RETURN_PARM};
use crate::model::*;
use mh_pak::Reader;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::rc::Rc;

/// The context an expression runs against: an object, a static library (Default__X) or nothing (null context)
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ctx {
    Obj(Id),
    Static,
    Null,
}

/// A pending latent action (UKismetSystemLibrary::Delay and friends): resume `func` on `obj` at `linkage`
#[derive(Clone, Debug)]
pub struct Latent {
    pub at: f64,
    pub obj: Id,
    pub func: String,
    /// -1: a timer (the function takes no parameters)
    pub linkage: i64,
    pub uuid: i64,
    /// looping timers: the period (0 = one shot)
    pub period: f64,
}

/// A playing widget animation (UUserWidget::PlayAnimation): ends at `end` (game time); finished events fire then
#[derive(Clone, Debug)]
pub struct Playing {
    pub widget: Id,
    pub anim: String,
    pub start: f64,
    pub end: f64,
    pub reverse: bool,
    /// 0 = loop forever (NumLoopsToPlay 0), else plays once per loop until `end`
    pub loops: i64,
    /// StartAtTime (seconds into the animation) and PlaybackSpeed
    pub offset: f64,
    pub speed: f64,
    /// the animation's length (seconds)
    pub len: f64,
}

/// What the widgets asked the host to do (natives that leave the UI: open a level, quit, console commands, sounds)
#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    OpenLevel { map: String, options: String },
    Console(String),
    Quit,
    Sound(String),
    /// UMordhauInput / UMordhauGameUserSettings were applied and saved: the game picks the new values up now
    /// (UMordhauInput::ApplySettings -> ForceRebuildKeymaps rva=0x1594240; UGameUserSettings::ApplySettings ->
    /// ApplyNonResolutionSettings rva=0x1582870)
    SettingsApplied { input: bool },
    /// (first-person r3) a call the UI makes on the owning pawn that the game must carry out: "RequestSuicide"
    /// (AAdvancedCharacter::RequestSuicide rva=0x149e680), "CycleCamera" (AMordhauCharacter::CycleCamera)
    Pawn(String),
}

struct Frame {
    this: Id,
    func: Rc<Func>,
    locals: Rc<RefCell<HashMap<String, V>>>,
    flow: Vec<u32>,
}

/// An l-value
#[derive(Clone, Debug)]
pub enum Place {
    Local(String),
    Member(Id, String),
    Persistent(Id, String),
    Field(Box<Place>, String),
    Elem(Box<Place>, usize),
    Temp,
}

pub struct Vm {
    pub rd: Reader,
    pub objs: Vec<Obj>,
    classes: HashMap<String, Rc<Class>>,
    pkg_json: HashMap<String, Rc<Vec<Value>>>,
    struct_fields: HashMap<String, Vec<String>>,
    /// AddToViewport widgets and their ZOrder, in insertion order
    pub viewport: Vec<(Id, i64)>,
    pub time: f64,
    pub latent: Vec<Latent>,
    pub playing: Vec<Playing>,
    pub actions: Vec<Action>,
    /// natives called that have no implementation (name -> count), for evidence / UNCONFIRMED lists
    pub missing: BTreeMap<String, usize>,
    /// (native, calling Blueprint class::function) pairs already logged (missing_log)
    pub missing_callers: std::collections::BTreeSet<(String, String)>,
    /// proxies the host fills (host.rs)
    pub world: HashMap<&'static str, Id>,
    libs: HashMap<String, Id>,
    pub focus: Option<Id>,
    /// texture metadata source (SetBrushFromTexture bMatchSize)
    pub src: Option<mh_assets::pak_source::PakSource>,
    steps: u64,
    depth: usize,
    /// $MH_UI_TRACE: print every executed statement (debugging)
    pub trace: bool,
    /// functions that ran into the per-frame step budget (a runaway loop: a missing native that a loop waits on)
    pub budget_hits: Vec<String>,
}

const STEP_BUDGET: u64 = 5_000_000;

impl Vm {
    pub fn new(rd: Reader) -> Vm {
        let mut vm = Vm {
            rd,
            objs: vec![Obj::default()], // id 0 = null
            classes: HashMap::new(),
            pkg_json: HashMap::new(),
            struct_fields: HashMap::new(),
            viewport: vec![],
            time: 0.0,
            latent: vec![],
            playing: vec![],
            actions: vec![],
            missing: BTreeMap::new(),
            missing_callers: std::collections::BTreeSet::new(),
            world: HashMap::new(),
            libs: HashMap::new(),
            focus: None,
            src: None,
            steps: 0,
            depth: 0,
            trace: false,
            budget_hits: vec![],
        };
        vm.objs[0].class = vm.native_class("Object");
        vm
    }

    pub fn o(&self, id: Id) -> &Obj {
        &self.objs[id as usize]
    }
    pub fn om(&mut self, id: Id) -> &mut Obj {
        &mut self.objs[id as usize]
    }
    pub fn alive(&self, id: Id) -> bool {
        id != 0 && (id as usize) < self.objs.len() && self.objs[id as usize].alive
    }
    pub fn prop(&self, id: Id, k: &str) -> V {
        self.o(id).get(k).clone()
    }
    pub fn set(&mut self, id: Id, k: &str, v: V) {
        self.om(id).props.insert(k.to_string(), v);
    }

    pub fn json(&mut self, pkg: &str) -> Option<Rc<Vec<Value>>> {
        if let Some(j) = self.pkg_json.get(pkg) {
            return Some(j.clone());
        }
        let j = Rc::new(self.rd.read(pkg)?);
        self.pkg_json.insert(pkg.to_string(), j.clone());
        Some(j)
    }

    pub fn native_class(&mut self, n: &str) -> Rc<Class> {
        if let Some(c) = self.classes.get(n) {
            return c.clone();
        }
        let c = Rc::new(Class { name: n.to_string(), cdo: native_defaults(n), ..Default::default() });
        self.classes.insert(n.to_string(), c.clone());
        c
    }

    /// A class by reference: "/Script/X" (or no package) -> native; a content package -> its Blueprint class
    pub fn class(&mut self, r: &ObjRef) -> Rc<Class> {
        if r.package.is_empty() || r.package.starts_with("/Script/") {
            return self.native_class(&r.name);
        }
        self.bp_class(&r.package).unwrap_or_else(|| self.native_class(&r.name))
    }

    /// The generated class of a Blueprint package
    pub fn bp_class(&mut self, pkg: &str) -> Option<Rc<Class>> {
        if let Some(c) = self.classes.get(pkg) {
            return Some(c.clone());
        }
        let pk = self.rd.open(pkg)?;
        let (ci, sup, members) = kismet::class_export(&self.rd, &pk)?;
        let name = pk.exports[ci].name.clone();
        let parent = sup.map(|s| self.class(&s));
        let funcs = kismet::class_functions(&self.rd, &pk, ci);
        let ex = self.json(pkg)?;
        let mut cdo: HashMap<String, V> = parent.as_ref().map(|p| p.cdo.clone()).unwrap_or_default();
        for m in &members {
            cdo.insert(m.name.clone(), zero(m));
        }
        let cdo_name = format!("Default__{name}");
        if let Some(p) = ex.iter().find(|e| e.get("Name").and_then(Value::as_str) == Some(&cdo_name)).and_then(|e| e.get("Properties")).and_then(Value::as_object) {
            for (k, v) in p {
                if k != "UberGraphFrame" {
                    let nv = json_v(v, &|_| None);
                    let nv = match cdo.get(k) {
                        Some(b) => merge_over(b, nv),
                        None => nv,
                    };
                    cdo.insert(k.clone(), nv);
                }
            }
        }
        let cls_json = ex.iter().find(|e| e.get("Name").and_then(Value::as_str) == Some(&name)).and_then(|e| e.get("Properties")).cloned().unwrap_or(Value::Null);
        let idx = |v: Option<&Value>| -> Option<usize> { v?.get("ObjectPath")?.as_str()?.rsplit_once('.')?.1.parse().ok() };
        let tree_root = idx(cls_json.get("WidgetTree")).and_then(|t| idx(ex.get(t)?.pointer("/Properties/RootWidget")));
        let mut bindings = vec![];
        for b in cls_json.get("Bindings").and_then(Value::as_array).into_iter().flatten() {
            let s = |k: &str| b.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            let path = b.pointer("/SourcePath/Segments").and_then(Value::as_array).map(|a| a.iter().filter_map(|x| x.get("Name").and_then(Value::as_str).map(str::to_string)).collect()).unwrap_or_default();
            let f = if s("Kind").ends_with("Function") { s("FunctionName") } else { String::new() };
            bindings.push((s("ObjectName"), s("PropertyName"), f, path));
        }
        let mut comp_delegates = vec![];
        for d in cls_json.get("DynamicBindingObjects").and_then(Value::as_array).into_iter().flatten() {
            if let Some(e) = idx(Some(d)).and_then(|i| ex.get(i)) {
                for b in e.pointer("/Properties/ComponentDelegateBindings").and_then(Value::as_array).into_iter().flatten() {
                    let s = |k: &str| b.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                    comp_delegates.push((s("ComponentPropertyName"), s("DelegatePropertyName"), s("FunctionNameToBind")));
                }
            }
        }
        let mut anims = HashMap::new();
        if tree_root.is_some() {
            if let Some(u) = mh_assets::umg::UmgPackage::read(&self.rd, pkg) {
                for a in &u.animations {
                    // MovieScene PlaybackRange (inclusive last tick) / TickResolution (umg.rs movie_scene_last_tick)
                    let len = if a.last_tick >= 0.0 && a.tick_resolution > 0.0 { a.last_tick / a.tick_resolution } else { 0.0 };
                    anims.insert(a.name.trim_end_matches("_INST").to_string(), len);
                }
            }
        }
        let anim_data = if tree_root.is_some() { crate::anim::read_package(&self.rd, pkg) } else { HashMap::new() };
        let c = Rc::new(Class {
            anim_data,
            name,
            package: pk.name.clone(),
            parent,
            funcs,
            members,
            cdo,
            exports: tree_root.map(|_| ex.clone()),
            tree_root,
            bindings,
            comp_delegates,
            anims,
        });
        self.classes.insert(pkg.to_string(), c.clone());
        Some(c)
    }

    pub fn new_obj(&mut self, class: Rc<Class>, name: &str) -> Id {
        let id = self.objs.len() as Id;
        self.objs.push(Obj { name: name.to_string(), class, alive: true, ..Default::default() });
        id
    }

    /// CreateWidget / NewObject of a Blueprint or native class; user widgets get their tree (UUserWidget::Initialize)
    pub fn create(&mut self, class: Rc<Class>, name: &str) -> Id {
        let id = self.new_obj(class, name);
        self.init_widget(id);
        id
    }

    fn class_of_json(&mut self, e: &Value) -> Rc<Class> {
        let c = e.get("Class").and_then(Value::as_str).unwrap_or("");
        let inner = c.split_once('\'').map(|x| x.1.trim_end_matches('\'')).unwrap_or(c);
        if c.starts_with("UScriptClass") || !inner.contains('/') {
            let n = e.get("Type").and_then(Value::as_str).unwrap_or(inner).to_string();
            return self.native_class(&n);
        }
        let pkg = inner.rsplit_once('.').map(|x| x.0).unwrap_or(inner);
        self.bp_class(pkg).unwrap_or_else(|| self.native_class(e.get("Type").and_then(Value::as_str).unwrap_or("Widget")))
    }

    /// UUserWidget::Initialize: duplicate the class's WidgetTree into this instance
    pub fn init_widget(&mut self, id: Id) {
        let cls = self.o(id).class.clone();
        let Some(tc) = cls.tree_class() else { return };
        let (Some(ex), Some(root)) = (tc.exports.clone(), tc.tree_root) else { return };
        let pkg = tc.package.clone();
        // pass 1: every export reachable from the root (widgets + slots) gets an object
        let mut map: HashMap<usize, Id> = HashMap::new();
        let idx = |v: Option<&Value>| -> Option<usize> { v?.get("ObjectPath")?.as_str()?.rsplit_once('.')?.1.parse().ok() };
        let mut stack = vec![root];
        let mut order = vec![];
        while let Some(i) = stack.pop() {
            if map.contains_key(&i) || order.len() > 5000 {
                continue;
            }
            let Some(e) = ex.get(i) else { continue };
            let c = self.class_of_json(e);
            let nm = e.get("Name").and_then(Value::as_str).unwrap_or("").to_string();
            let oid = self.new_obj(c, &nm);
            map.insert(i, oid);
            order.push(i);
            if let Some(p) = e.get("Properties") {
                for s in p.get("Slots").and_then(Value::as_array).into_iter().flatten() {
                    if let Some(si) = idx(Some(s)) {
                        stack.push(si);
                        if let Some(ci) = idx(ex.get(si).and_then(|x| x.pointer("/Properties/Content"))) {
                            stack.push(ci);
                        }
                    }
                }
                // a nested user widget's NamedSlotBindings: the content widgets live in this (outer) tree
                for b in p.get("NamedSlotBindings").and_then(Value::as_array).into_iter().flatten() {
                    if let Some(ci) = idx(b.get("Content")) {
                        stack.push(ci);
                    }
                }
            }
        }
        // pass 2: properties (in-package references -> the new objects), structure, owner, variable binding
        let local = |op: &str| -> Option<Id> {
            let (p, n) = op.rsplit_once('.')?;
            if p != pkg {
                return None;
            }
            map.get(&n.parse::<usize>().ok()?).copied()
        };
        for &i in &order {
            let e = &ex[i];
            let oid = map[&i];
            let mut props: HashMap<String, V> = HashMap::new();
            let arch = self.objs[oid as usize].class.clone();
            if let Some(p) = e.get("Properties").and_then(Value::as_object) {
                for (k, v) in p {
                    // partial structs over the archetype's (the widget class defaults)
                    let nv = json_v(v, &local);
                    let nv = match arch.cdo.get(k) {
                        Some(b @ V::Struct(_)) => merge_over(b, nv),
                        _ => nv,
                    };
                    props.insert(k.clone(), nv);
                }
            }
            let slots: Vec<Id> = props.get("Slots").map(|s| s.arr().iter().filter_map(V::obj).collect()).unwrap_or_default();
            let content = props.get("Content").and_then(V::obj);
            let ps = props.get("Slot").and_then(V::obj);
            let o = self.om(oid);
            o.props = props;
            o.slots = slots;
            o.owner = Some(id);
            if content.is_some() {
                o.content = content;
            }
            if ps.is_some() {
                o.parent_slot = ps;
            }
        }
        for &i in &order {
            let oid = map[&i];
            let slots = self.o(oid).slots.clone();
            for s in slots {
                self.om(s).panel = Some(oid);
                if let Some(c) = self.o(s).content {
                    self.om(c).parent_slot = Some(s);
                }
            }
            let nm = self.o(oid).name.clone();
            // widgets are the tree's variables, panel slots are not (UNamedSlot is a widget despite its name)
            if !self.o(oid).class.isa("PanelSlot") {
                self.om(id).props.insert(nm, V::Obj(oid));
            }
        }
        let r = map[&root];
        self.om(id).root = Some(r);
        // nested user widgets: their own trees (template overrides are already their props)
        for &i in &order {
            let oid = map[&i];
            if self.o(oid).class.tree_class().is_some() {
                self.init_widget(oid);
            }
        }
        // UUserWidget::Initialize -> NamedSlotBindings: each binding's content (a widget of this tree) goes into the
        // nested user widget's UNamedSlot of that name (UUserWidget::SetContentForSlot)
        for &i in &order {
            let oid = map[&i];
            for b in self.prop(oid, "NamedSlotBindings").arr().to_vec() {
                let (name, content) = (b.field("Name").s(), b.field("Content").obj());
                let (Some(content), Some(slot)) = (content, self.child(oid, &name)) else { continue };
                crate::natives::add_child(self, slot, content);
            }
        }
        // ComponentDelegateBindings (UWidgetBlueprintGeneratedClass::BindDynamicDelegates)
        let mut c = Some(cls.clone());
        while let Some(k) = c {
            for (comp, del, func) in &k.comp_delegates {
                if let Some(w) = self.o(id).props.get(comp).and_then(V::obj) {
                    let mut m = match self.prop(w, del) {
                        V::Multi(m) => m,
                        _ => vec![],
                    };
                    m.push((id, func.clone()));
                    self.set(w, del, V::Multi(m));
                }
            }
            c = k.parent.clone();
        }
        // widget animation objects as variables (UWidgetAnimation properties named after the animation)
        let names: Vec<String> = cls.tree_class().map(|t| t.anims.keys().cloned().collect()).unwrap_or_default();
        for a in names {
            let ar = std::sync::Arc::new(ObjRef { package: pkg.clone(), name: a.clone(), class: "WidgetAnimation".into(), outer: String::new() });
            self.om(id).props.insert(a, V::Asset(ar));
        }
        // OnInitialized (UUserWidget::Initialize -> NativeOnInitialized)
        self.event(id, "OnInitialized", vec![]);
    }

    /// UUserWidget::OnWidgetRebuilt -> NativePreConstruct + NativeConstruct, children first (their Slate widgets are
    /// built during the parent's RebuildWidget)
    pub fn construct(&mut self, id: Id) {
        if !self.alive(id) || self.o(id).constructed {
            return;
        }
        self.om(id).constructed = true;
        let mut kids = vec![];
        self.descendants(id, &mut kids);
        for k in kids {
            if k != id && self.o(k).class.tree_class().is_some() && !self.o(k).constructed {
                self.construct(k);
            }
        }
        self.event(id, "PreConstruct", vec![V::Bool(false)]);
        self.event(id, "Construct", vec![]);
    }

    /// widgets under `id` (through user-widget roots, panel slots, content)
    /// (perf) the widget tree under `id` (user widget roots, panel slots, list entries), depth first; already listed
    /// widgets (in `out` or found earlier) are not walked again. A visited set replaces the linear `out.contains`
    /// scans (quadratic in the tree size: most of run_bindings' 40 ms a frame in the match HUD)
    pub fn descendants(&self, id: Id, out: &mut Vec<Id>) {
        let mut seen: HashSet<Id> = out.iter().copied().collect();
        self.descendants_rec(id, out, &mut seen);
    }

    fn descendants_rec(&self, id: Id, out: &mut Vec<Id>, seen: &mut HashSet<Id>) {
        if out.len() > 100_000 {
            return;
        }
        out.push(id);
        seen.insert(id);
        let o = self.o(id);
        if let Some(r) = o.root {
            if seen.contains(&r) {
                return;
            }
            self.descendants_rec(r, out, seen);
        }
        for &s in &o.slots {
            if let Some(c) = self.o(s).content {
                if !seen.contains(&c) {
                    self.descendants_rec(c, out, seen);
                }
            }
        }
        // a UListView / UTileView's generated entry widgets (SListView children: Slate's widget tree, so property
        // bindings, ticks and searches reach them as in the game)
        if let Some(V::Array(es)) = o.props.get("__entries") {
            for e in es.iter().filter_map(V::obj) {
                if self.alive(e) && !seen.contains(&e) {
                    self.descendants_rec(e, out, seen);
                }
            }
        }
    }

    /// The widget with `name` inside user widget `id`'s tree (its variable)
    pub fn child(&self, id: Id, name: &str) -> Option<Id> {
        self.o(id).props.get(name).and_then(V::obj)
    }

    /// Slate SetUserFocus searches an attached EVisibility::Visible path (original RVA 0x1cdf950).
    /// Hidden/collapsed ancestors and inactive switcher children are excluded; clipping, screen position and
    /// hit-test visibility do not prevent keyboard focus. Generated list entries are structural children too.
    pub fn visible_widget_path(&self, target: Id) -> Option<Vec<Id>> {
        fn find(vm: &Vm, id: Id, target: Id, path: &mut Vec<Id>, seen: &mut HashSet<Id>) -> bool {
            if !vm.alive(id) || path.len() >= 256 || matches!(vm.prop(id, "Visibility").i(), 1 | 2) || !seen.insert(id) {
                return false;
            }
            path.push(id);
            if id == target {
                return true;
            }
            let o = vm.o(id);
            if let Some(root) = o.root {
                if find(vm, root, target, path, seen) {
                    return true;
                }
            }
            if o.class.native() == "WidgetSwitcher" {
                let active = usize::try_from(vm.prop(id, "ActiveWidgetIndex").i()).ok();
                if let Some(child) = active.and_then(|i| o.slots.get(i)).and_then(|&s| vm.o(s).content) {
                    if find(vm, child, target, path, seen) {
                        return true;
                    }
                }
            } else {
                for child in o.slots.iter().filter_map(|&s| vm.o(s).content) {
                    if find(vm, child, target, path, seen) {
                        return true;
                    }
                }
            }
            for entry in vm.prop(id, "__entries").arr().iter().filter_map(V::obj) {
                if find(vm, entry, target, path, seen) {
                    return true;
                }
            }
            path.pop();
            false
        }
        let mut seen = HashSet::new();
        let mut path = Vec::new();
        for &(root, _) in self.viewport.iter().rev() {
            if find(self, root, target, &mut path, &mut seen) {
                return Some(path);
            }
        }
        None
    }

    // ---- execution -------------------------------------------------------------------------------------------------

    /// Call a function by name on an object (virtual dispatch through the Blueprint chain, then natives);
    /// returns (return value, out parameters by parameter name)
    pub fn call_named(&mut self, id: Id, name: &str, args: Vec<V>) -> (V, HashMap<String, V>) {
        if !self.alive(id) {
            return (V::None, HashMap::new());
        }
        let cls = self.o(id).class.clone();
        if let Some(f) = cls.func(name) {
            return self.run(id, f, args);
        }
        let r = crate::natives::call(self, Ctx::Obj(id), "", name, &args);
        (r.map(|x| x.ret).unwrap_or_default(), HashMap::new())
    }

    /// A Blueprint event (no-op when the class does not implement it)
    pub fn event(&mut self, id: Id, name: &str, args: Vec<V>) -> Option<HashMap<String, V>> {
        if !self.alive(id) {
            return None;
        }
        let f = self.o(id).class.func(name)?;
        // a native event's result (UUserWidget::OnPreviewKeyDown / OnKeyDown / OnMouseButtonDown: FEventReply) is a
        // CPF_ReturnParm, not an out parameter: hand it back under "ReturnValue" with the outs
        let (ret, mut outs) = self.run(id, f, args);
        if !matches!(ret, V::None) {
            outs.entry("ReturnValue".to_string()).or_insert(ret);
        }
        Some(outs)
    }

    pub fn has_func(&self, id: Id, name: &str) -> bool {
        self.alive(id) && self.o(id).class.func(name).is_some()
    }

    fn run(&mut self, this: Id, f: Rc<Func>, args: Vec<V>) -> (V, HashMap<String, V>) {
        if self.depth > 200 {
            return (V::None, HashMap::new());
        }
        let uber = f.name.starts_with("ExecuteUbergraph_");
        let locals = if uber { self.o(this).uber.clone() } else { Rc::new(RefCell::new(HashMap::new())) };
        {
            let mut l = locals.borrow_mut();
            for p in &f.props {
                if !(uber && l.contains_key(&p.name)) {
                    l.insert(p.name.clone(), zero(p));
                }
            }
            for (p, a) in f.params().zip(args) {
                l.insert(p.name.clone(), a);
            }
        }
        let mut fr = Frame { this, func: f.clone(), locals: locals.clone(), flow: vec![] };
        self.depth += 1;
        self.exec(&mut fr);
        self.depth -= 1;
        let l = locals.borrow();
        let mut outs = HashMap::new();
        let mut ret = V::None;
        for p in f.params() {
            if p.flags & CPF_RETURN_PARM != 0 {
                ret = l.get(&p.name).cloned().unwrap_or_default();
            } else if p.flags & CPF_OUT_PARM != 0 {
                outs.insert(p.name.clone(), l.get(&p.name).cloned().unwrap_or_default());
            }
        }
        // Blueprint functions return through their out parameters; "ReturnValue" is the conventional single result
        if matches!(ret, V::None) {
            if let Some(r) = outs.get("ReturnValue") {
                ret = r.clone();
            }
        }
        (ret, outs)
    }

    fn exec(&mut self, fr: &mut Frame) {
        let code = fr.func.clone();
        let mut pc = 0usize;
        while pc < code.code.len() {
            self.steps += 1;
            if self.steps > STEP_BUDGET {
                if self.steps == STEP_BUDGET + 1 {
                    self.budget_hits.push(format!("{}@{}", code.name, code.code.get(pc).map_or(0, |c| c.0)));
                }
                return;
            }
            let (at, ex) = &code.code[pc];
            if self.trace {
                eprintln!("{}@{} {:?}", code.name, at, ex);
            }
            pc += 1;
            let goto = |off: u32| -> Option<usize> { code.index.get(&off).copied() };
            match ex {
                Ex::Jump(off) => match goto(*off) {
                    Some(p) => pc = p,
                    None => return,
                },
                Ex::JumpIfNot(off, c) => {
                    if !self.eval(fr, Ctx::Obj(fr.this), c).truthy() {
                        match goto(*off) {
                            Some(p) => pc = p,
                            None => return,
                        }
                    }
                }
                Ex::PushFlow(off) => fr.flow.push(*off),
                Ex::Op(0x4D) => match fr.flow.pop().and_then(goto) {
                    Some(p) => pc = p,
                    None => return,
                },
                Ex::PopFlowIfNot(c) => {
                    if !self.eval(fr, Ctx::Obj(fr.this), c).truthy() {
                        match fr.flow.pop().and_then(goto) {
                            Some(p) => pc = p,
                            None => return,
                        }
                    }
                }
                Ex::ComputedJump(e) => {
                    let off = self.eval(fr, Ctx::Obj(fr.this), e).i();
                    match goto(off as u32) {
                        Some(p) => pc = p,
                        None => return,
                    }
                }
                Ex::Return(e) => {
                    self.eval(fr, Ctx::Obj(fr.this), e);
                    return;
                }
                Ex::Op(0x53) => return,
                e => {
                    self.eval(fr, Ctx::Obj(fr.this), e);
                }
            }
        }
    }

    fn get_place(&self, fr: &Frame, p: &Place) -> V {
        match p {
            Place::Local(n) => fr.locals.borrow().get(n).cloned().unwrap_or_default(),
            Place::Member(id, n) => self.prop(*id, n),
            Place::Persistent(id, n) => self.o(*id).uber.borrow().get(n).cloned().unwrap_or_default(),
            Place::Field(b, n) => self.get_place(fr, b).field(n).clone(),
            Place::Elem(b, i) => self.get_place(fr, b).arr().get(*i).cloned().unwrap_or_default(),
            Place::Temp => V::None,
        }
    }

    fn set_place(&mut self, fr: &Frame, p: &Place, v: V) {
        match p {
            Place::Local(n) => {
                fr.locals.borrow_mut().insert(n.clone(), v);
            }
            Place::Member(id, n) => {
                if self.alive(*id) {
                    self.set(*id, n, v)
                }
            }
            Place::Persistent(id, n) => {
                self.o(*id).uber.borrow_mut().insert(n.clone(), v);
            }
            Place::Field(b, n) => {
                let mut s = self.get_place(fr, b);
                if !matches!(s, V::Struct(_)) {
                    s = V::Struct(Box::default());
                }
                if let V::Struct(m) = &mut s {
                    m.insert(n.clone(), v);
                }
                self.set_place(fr, b, s);
            }
            Place::Elem(b, i) => {
                let mut a = self.get_place(fr, b);
                if let V::Array(x) = &mut a {
                    if *i < x.len() {
                        x[*i] = v;
                    }
                }
                self.set_place(fr, b, a);
            }
            Place::Temp => {}
        }
    }

    fn lvalue(&mut self, fr: &mut Frame, ctx: Ctx, e: &Ex) -> Place {
        match e {
            Ex::Var(0x00 | 0x48, n) => Place::Local(n.clone()),
            Ex::Var(_, n) => match ctx {
                Ctx::Obj(id) => Place::Member(id, n.clone()),
                _ => Place::Temp,
            },
            Ex::Context(_, o, _, _, inner) => match self.eval(fr, ctx, o) {
                V::Obj(id) if self.alive(id) => self.lvalue(fr, Ctx::Obj(id), inner),
                _ => Place::Temp,
            },
            Ex::StructMember(f, inner) => {
                let b = self.lvalue(fr, ctx, inner);
                Place::Field(Box::new(b), f.rsplit('.').next().unwrap_or(f).to_string())
            }
            Ex::ArrayGetByRef(a, i) => {
                let b = self.lvalue(fr, ctx, a);
                let i = self.eval(fr, Ctx::Obj(fr.this), i).i().max(0) as usize;
                Place::Elem(Box::new(b), i)
            }
            Ex::Skip(_, inner) | Ex::InterfaceContext(inner) => self.lvalue(fr, ctx, inner),
            _ => Place::Temp,
        }
    }

    fn struct_fields(&mut self, r: &ObjRef) -> Vec<String> {
        if let Some(f) = self.struct_fields.get(&r.name) {
            return f.clone();
        }
        let s = |a: &[&str]| a.iter().map(|x| x.to_string()).collect::<Vec<_>>();
        // UE 4.26 USTRUCT UPROPERTY declaration order (CoreUObject NoExportTypes.h, SlateCore / UMG headers)
        let f = match r.name.as_str() {
            "LinearColor" => s(&["R", "G", "B", "A"]),
            "Color" => s(&["B", "G", "R", "A"]),
            "Vector2D" | "IntPoint" => s(&["X", "Y"]),
            "Vector" | "IntVector" => s(&["X", "Y", "Z"]),
            "Vector4" | "Quat" => s(&["X", "Y", "Z", "W"]),
            "Rotator" => s(&["Pitch", "Yaw", "Roll"]),
            "Margin" => s(&["Left", "Top", "Right", "Bottom"]),
            "SlateColor" => s(&["SpecifiedColor", "ColorUseRule"]),
            "SlateBrush" => s(&["ImageSize", "Margin", "TintColor", "ResourceObject", "ResourceName", "UVRegion", "DrawAs", "Tiling", "Mirroring", "ImageType", "bIsDynamicallyLoaded", "bHasUObject"]),
            "Box2D" => s(&["Min", "Max", "bIsValid"]),
            "LatentActionInfo" => s(&["Linkage", "UUID", "ExecutionFunction", "CallbackTarget"]),
            "Key" => s(&["KeyName"]),
            "SlateSound" => s(&["ResourceObject"]),
            "ButtonStyle" => s(&["Normal", "Hovered", "Pressed", "Disabled", "NormalPadding", "PressedPadding", "PressedSlateSound", "HoveredSlateSound"]),
            "SlateFontInfo" => s(&["FontObject", "FontMaterial", "OutlineSettings", "TypefaceFontName", "Size", "LetterSpacing"]),
            "FontOutlineSettings" => s(&["OutlineSize", "bSeparateFillAlpha", "bApplyOutlineToDropShadows", "OutlineMaterial", "OutlineColor"]),
            "Anchors" => s(&["Minimum", "Maximum"]),
            "AnchorData" => s(&["Offsets", "Anchors", "Alignment"]),
            "WidgetTransform" => s(&["Translation", "Scale", "Shear", "Angle"]),
            "Transform" => s(&["Rotation", "Translation", "Scale3D"]),
            "DateTime" | "Timespan" => s(&["Ticks"]),
            "ProgressBarStyle" => s(&["BackgroundImage", "FillImage", "MarqueeImage"]),
            "CheckBoxStyle" => s(&["CheckBoxType", "UncheckedImage", "UncheckedHoveredImage", "UncheckedPressedImage", "CheckedImage", "CheckedHoveredImage", "CheckedPressedImage", "UndeterminedImage", "UndeterminedHoveredImage", "UndeterminedPressedImage", "Padding", "ForegroundColor", "HoveredForeground", "PressedForeground", "CheckedForeground", "CheckedHoveredForeground", "CheckedPressedForeground", "UndeterminedForeground", "BorderBackgroundColor", "CheckedSlateSound", "UncheckedSlateSound", "HoveredSlateSound"]),
            _ if !r.package.starts_with("/Script") && !r.package.is_empty() => kismet::user_struct_fields(&self.rd, &r.package).unwrap_or_default(),
            _ => vec![],
        };
        self.struct_fields.insert(r.name.clone(), f.clone());
        f
    }

    fn text_lit(&mut self, fr: &mut Frame, t: &kismet::TextLit) -> String {
        use kismet::TextLit as T;
        match t {
            T::Empty => String::new(),
            T::Localized(s, _, _) | T::Invariant(s) | T::Literal(s) => self.eval(fr, Ctx::Obj(fr.this), s).s(),
            T::StringTable(_, k) => self.eval(fr, Ctx::Obj(fr.this), k).s(),
        }
    }

    fn eval(&mut self, fr: &mut Frame, ctx: Ctx, e: &Ex) -> V {
        match e {
            Ex::Var(..) | Ex::StructMember(..) | Ex::ArrayGetByRef(..) => {
                let p = self.lvalue(fr, ctx, e);
                self.get_place(fr, &p)
            }
            Ex::Let(_, var, val) | Ex::LetKind(_, var, val) => {
                let v = self.eval(fr, ctx, val);
                let p = self.lvalue(fr, ctx, var);
                self.set_place(fr, &p, v);
                V::None
            }
            Ex::LetPersistent(n, val) => {
                let v = self.eval(fr, ctx, val);
                let this = fr.this;
                self.set_place(fr, &Place::Persistent(this, n.rsplit('.').next().unwrap_or(n).to_string()), v);
                V::None
            }
            Ex::Context(_, o, _, _, inner) => match self.eval(fr, ctx, o) {
                V::Obj(id) if self.alive(id) => self.eval(fr, Ctx::Obj(id), inner),
                // Default__BP_X_C of a Blueprint function library: its default object runs the call
                // Default__BP_X_C (function library) or a Blueprint class value (TSubclassOf member access reads the
                // class default object)
                V::Asset(a) if !a.package.starts_with("/Script/") && !a.package.is_empty() && (a.name.starts_with("Default__") || a.name.ends_with("_C") || a.class.contains("Class") || a.name.is_empty()) => match self.library(&a.package) {
                    Some(id) => self.eval(fr, Ctx::Obj(id), inner),
                    None => self.eval(fr, Ctx::Static, inner),
                },
                V::Asset(_) => self.eval(fr, Ctx::Static, inner),
                _ => {
                    // null context: the call is skipped (FFrame "Accessed None", result zeroed); a native static
                    // library call through Default__ objects is never null
                    if let Ex::Final(_, Some(f), _) = &**inner {
                        if f.package.starts_with("/Script/") && f.outer.ends_with("Library") {
                            return self.eval(fr, Ctx::Static, inner);
                        }
                    }
                    V::None
                }
            },
            Ex::InterfaceContext(inner) | Ex::Skip(_, inner) => self.eval(fr, ctx, inner),
            Ex::Virtual(_, name, args) => self.call(fr, ctx, name, None, args),
            Ex::Final(_, f, args) => {
                let Some(f) = f.clone() else { return V::None };
                self.call(fr, ctx, &f.name.clone(), Some(&f), args)
            }
            Ex::Int(i) | Ex::IntConst8(i) | Ex::Int64(i) => V::Int(*i),
            Ex::UInt64(u) => V::Int(*u as i64),
            Ex::Float(f) | Ex::Double(f) => V::Float(*f),
            Ex::Byte(b) => V::Int(*b as i64),
            Ex::Str(s) => V::Str(s.clone()),
            Ex::Name(n) => V::Name(n.clone()),
            Ex::Obj(o) => match o {
                Some(r) => V::Asset(std::sync::Arc::new(r.clone())),
                None => V::None,
            },
            Ex::Vec3(0x22, v) => V::st(&[("Pitch", V::Float(v[0])), ("Yaw", V::Float(v[1])), ("Roll", V::Float(v[2]))]),
            Ex::Vec3(_, v) => V::st(&[("X", V::Float(v[0])), ("Y", V::Float(v[1])), ("Z", V::Float(v[2]))]),
            Ex::Transform(_) => V::Struct(Box::default()),
            Ex::Text(t) => V::Text(self.text_lit(fr, t)),
            Ex::Op(0x17) => match ctx {
                Ctx::Obj(id) => V::Obj(id),
                _ => V::Obj(fr.this),
            },
            Ex::Op(0x25) => V::Int(0),
            Ex::Op(0x26) => V::Int(1),
            Ex::Op(0x27) => V::Bool(true),
            Ex::Op(0x28) => V::Bool(false),
            Ex::Op(_) => V::None,
            Ex::BitFieldConst(_, b) => V::Bool(*b != 0),
            Ex::Cast(t, c, inner) => {
                let v = self.eval(fr, ctx, inner);
                match (t, &v, c) {
                    // a loaded package object: its class from the package (Texture2D, ...) when known
                    (0x2E, V::Asset(a), Some(c)) if !a.class.is_empty() && a.class != "BlueprintGeneratedClass" => {
                        if a.class == c.name || (c.name == "Texture" && a.class.starts_with("Texture")) || c.name == "Object" {
                            v
                        } else {
                            V::None
                        }
                    }
                    (0x2E, V::Obj(id), Some(c)) => {
                        if self.alive(*id) && self.o(*id).class.isa(&c.name) {
                            v
                        } else {
                            V::None
                        }
                    }
                    _ => v,
                }
            }
            Ex::PrimitiveCast(c, inner) => {
                let v = self.eval(fr, ctx, inner);
                match c {
                    // CST_ObjectToBool 0x47, CST_InterfaceToBool 0x49 (UE 4.26 ECastToken)
                    0x47 | 0x49 => V::Bool(match &v {
                        V::Obj(id) => self.alive(*id),
                        V::None => false,
                        _ => true,
                    }),
                    _ => v,
                }
            }
            Ex::StructConst(st, items) => {
                let names = st.as_ref().map(|s| self.struct_fields(s)).unwrap_or_default();
                let mut m = BTreeMap::new();
                for (i, it) in items.iter().enumerate() {
                    let v = self.eval(fr, ctx, it);
                    m.insert(names.get(i).cloned().unwrap_or_else(|| format!("_{i}")), v);
                }
                V::Struct(Box::new(m))
            }
            Ex::SetArray(target, items) => {
                let vs: Vec<V> = items.iter().map(|x| self.eval(fr, ctx, x)).collect();
                let p = self.lvalue(fr, ctx, target);
                self.set_place(fr, &p, V::Array(vs));
                V::None
            }
            // EX_SetSet 0x39 / EX_SetMap 0x3B: assign the literal to the target (a map's items are key, value pairs)
            Ex::SetColl(t, target, items) => {
                let vs: Vec<V> = items.iter().map(|x| self.eval(fr, ctx, x)).collect();
                let v = if *t == 0x3B { V::Map(vs.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0].clone(), c[1].clone())).collect()) } else { V::Array(vs) };
                let p = self.lvalue(fr, ctx, target);
                self.set_place(fr, &p, v);
                V::None
            }
            Ex::ConstColl(0x3F, _, items) => {
                let vs: Vec<V> = items.iter().map(|x| self.eval(fr, ctx, x)).collect();
                V::Map(vs.chunks(2).filter(|c| c.len() == 2).map(|c| (c[0].clone(), c[1].clone())).collect())
            }
            Ex::ConstColl(_, _, items) => V::Array(items.iter().map(|x| self.eval(fr, ctx, x)).collect()),
            Ex::InstanceDelegate(n) => V::Delegate(fr.this, n.clone()),
            Ex::BindDelegate(fname, d, o) => {
                let target = self.eval(fr, ctx, o).obj().unwrap_or(0);
                let p = self.lvalue(fr, ctx, d);
                self.set_place(fr, &p, V::Delegate(target, fname.clone()));
                V::None
            }
            Ex::MultiOp(t, d, v) => {
                let add = self.eval(fr, ctx, v);
                let p = self.lvalue(fr, ctx, d);
                let mut cur = match self.get_place(fr, &p) {
                    V::Multi(m) => m,
                    _ => vec![],
                };
                if let V::Delegate(o, n) = add {
                    cur.retain(|(a, b)| !(*a == o && *b == n));
                    if *t == 0x5C {
                        cur.push((o, n));
                    }
                }
                self.set_place(fr, &p, V::Multi(cur));
                V::None
            }
            Ex::ClearMulticast(d) => {
                let p = self.lvalue(fr, ctx, d);
                self.set_place(fr, &p, V::Multi(vec![]));
                V::None
            }
            Ex::CallMulticast(_, d, args) => {
                let list = match self.eval(fr, ctx, d) {
                    V::Multi(m) => m,
                    _ => vec![],
                };
                let vs: Vec<V> = args.iter().map(|x| self.eval(fr, Ctx::Obj(fr.this), x)).collect();
                for (o, n) in list {
                    self.call_named(o, &n, vs.clone());
                }
                V::None
            }
            Ex::Switch(_, idx, cases, default) => {
                let v = self.eval(fr, ctx, idx);
                for (c, r) in cases {
                    let cv = self.eval(fr, ctx, c);
                    if cv.same(&v) {
                        return self.eval(fr, ctx, r);
                    }
                }
                self.eval(fr, ctx, default)
            }
            Ex::SoftObjectConst(inner) => {
                let s = self.eval(fr, ctx, inner).s();
                let (p, n) = s.rsplit_once('.').unwrap_or((&s, ""));
                V::Asset(std::sync::Arc::new(ObjRef { package: kismet::content_path(p), name: n.to_string(), class: String::new(), outer: String::new() }))
            }
            Ex::PropertyConst(n) => V::Name(n.clone()),
            Ex::FieldPathConst(inner) => self.eval(fr, ctx, inner),
            Ex::SkipOffsetConst(o) => V::Int(*o as i64),
            Ex::Assert(_) | Ex::Instrumentation => V::None,
            _ => V::None,
        }
    }

    /// A call expression: Blueprint function (by name in the context object's class chain, or the specific class of
    /// a final call) or a native
    fn call(&mut self, fr: &mut Frame, ctx: Ctx, name: &str, f: Option<&ObjRef>, args: &[Ex]) -> V {
        let this_ctx = Ctx::Obj(fr.this);
        let target = match ctx {
            Ctx::Obj(id) => Some(id),
            _ => None,
        };
        // a Blueprint function library (Default__BP_X_C.Func): run it on the library class's default object
        if target.is_none() {
            if let Some(r) = f {
                if !r.package.starts_with("/Script/") && !r.package.is_empty() {
                    if let Some(id) = self.library(&r.package) {
                        if let Some(func) = self.o(id).class.func(name) {
                            return self.call_bp(fr, id, func, args);
                        }
                    }
                }
            }
        }
        let bpf = match (target, f) {
            (Some(id), Some(r)) if !r.package.starts_with("/Script/") && !r.package.is_empty() => {
                // a final call to a Blueprint function: that class's version (Super calls), else the chain's
                let mut c = Some(self.o(id).class.clone());
                let mut found = None;
                while let Some(k) = c {
                    if k.name == r.outer {
                        found = k.func(name);
                        break;
                    }
                    c = k.parent.clone();
                }
                found.or_else(|| self.o(id).class.func(name))
            }
            (Some(id), None) => self.o(id).class.func(name),
            _ => None,
        };
        if let (Some(id), Some(func)) = (target, bpf) {
            return self.call_bp(fr, id, func, args);
        }
        let vals: Vec<V> = args.iter().map(|a| self.eval(fr, this_ctx, a)).collect();
        let class = f.map(|r| r.outer.clone()).unwrap_or_default();
        match crate::natives::call(self, ctx, &class, name, &vals) {
            Some(r) => {
                for (i, v) in r.outs {
                    if let Some(a) = args.get(i) {
                        let pl = self.lvalue(fr, this_ctx, a);
                        self.set_place(fr, &pl, v);
                    }
                }
                r.ret
            }
            None => {
                let key = format!("{class}::{name}");
                *self.missing.entry(key.clone()).or_default() += 1;
                let caller = format!("{}::{}", self.o(fr.this).class.name, fr.func.name);
                if self.missing_callers.insert((key.clone(), caller.clone())) {
                    missing_log(&key, &caller);
                }
                V::None
            }
        }
    }

    fn call_bp(&mut self, fr: &mut Frame, id: Id, func: Rc<Func>, args: &[Ex]) -> V {
        let this_ctx = Ctx::Obj(fr.this);
        let params: Vec<_> = func.params().cloned().collect();
        let vals: Vec<V> = args.iter().map(|a| self.eval(fr, this_ctx, a)).collect();
        let (ret, outs) = self.run(id, func, vals);
        for (i, p) in params.iter().enumerate() {
            if p.flags & CPF_OUT_PARM != 0 && p.flags & CPF_RETURN_PARM == 0 && p.flags & CPF_PARM != 0 {
                if let (Some(a), Some(v)) = (args.get(i), outs.get(&p.name)) {
                    let pl = self.lvalue(fr, this_ctx, a);
                    self.set_place(fr, &pl, v.clone());
                }
            }
        }
        ret
    }

    /// the default object of a Blueprint function library / macro library class (one per package)
    pub fn library(&mut self, pkg: &str) -> Option<Id> {
        let key = format!("__lib:{pkg}");
        if let Some(&id) = self.libs.get(&key) {
            return Some(id);
        }
        let c = self.bp_class(pkg)?;
        let n = format!("Default__{}", c.name);
        let id = self.new_obj(c, &n);
        self.libs.insert(key, id);
        Some(id)
    }

    /// the position (seconds into the animation) of a playing animation at `now`
    fn anim_pos(p: &Playing, now: f64) -> f64 {
        let mut e = (now - p.start) * p.speed + p.offset;
        if p.len > 0.0 {
            e = if p.loops == 0 || now < p.end { e % p.len.max(1e-6) } else { p.len };
            if now >= p.end && p.loops != 0 {
                e = p.len;
            }
        }
        if p.reverse {
            p.len - e
        } else {
            e
        }
    }

    /// evaluate every playing widget animation (UUMGSequencePlayer::Tick -> the movie scene's property tracks)
    pub fn apply_animations(&mut self) {
        let now = self.time;
        let playing = self.playing.clone();
        for p in playing {
            if !self.alive(p.widget) {
                continue;
            }
            let data = self.o(p.widget).class.tree_class().and_then(|c| c.anim_data.get(&p.anim).cloned());
            if let Some(a) = data {
                let pos = Self::anim_pos(&p, now.min(p.end.max(p.start)));
                crate::anim::apply(self, p.widget, &a, pos);
            }
        }
    }

    /// Advance time: latent actions and animation ends (their finished events)
    pub fn tick(&mut self, dt: f64) {
        self.time += dt;
        self.steps = 0;
        let now = self.time;
        let due: Vec<Latent> = self.latent.iter().filter(|l| l.at <= now).cloned().collect();
        self.latent.retain(|l| l.at > now);
        for l in due {
            if l.period > 0.0 {
                let mut n = l.clone();
                n.at = l.at + l.period;
                self.latent.push(n);
            }
            if let Some(d) = l.func.strip_prefix("__broadcast:") {
                // a deferred native completion (natives.rs SendHTTPRequest): failure arguments
                if let V::Multi(m) = self.prop(l.obj, d) {
                    for (o, f) in m {
                        self.call_named(o, &f, vec![V::Struct(Box::default()), V::Int(0), V::Float(0.0)]);
                    }
                }
                continue;
            }
            let args = if l.linkage >= 0 { vec![V::Int(l.linkage)] } else { vec![] };
            self.call_named(l.obj, &l.func, args);
        }
        self.apply_animations();
        let done: Vec<Playing> = self.playing.iter().filter(|p| p.end <= now && p.loops != 0).cloned().collect();
        self.playing.retain(|p| !(p.end <= now && p.loops != 0));
        for p in done {
            // UUserWidget::OnAnimationFinished (BlueprintNativeEvent) + bound delegates (BindToAnimationFinished)
            let a = V::Asset(std::sync::Arc::new(ObjRef { package: String::new(), name: p.anim.clone(), class: "WidgetAnimation".into(), outer: String::new() }));
            self.event(p.widget, "OnAnimationFinished", vec![a]);
            let key = format!("__anim_finished:{}", p.anim);
            if let V::Multi(m) = self.prop(p.widget, &key) {
                for (o, n) in m {
                    self.call_named(o, &n, vec![]);
                }
            }
        }
    }

    /// Widget animation progress 0..1 for (widget, animation) if playing (for render-time effects)
    pub fn anim_t(&self, widget: Id, anim: &str) -> Option<f64> {
        let p = self.playing.iter().find(|p| p.widget == widget && p.anim == anim)?;
        let t = ((self.time - p.start) / (p.end - p.start).max(1e-6)).clamp(0.0, 1.0);
        Some(if p.reverse { 1.0 - t } else { t })
    }
}

/// UMG widget constructor defaults the packages leave out (delta-serialized against them). UE 4.26 values from the
/// UMG constructors (`UBorder::UBorder` 0x2ba84b0, `UImage::UImage` 0x2baa620, `UButton::UButton` 0x2ba8740,
/// `UProgressBar::UProgressBar` 0x2bac450, `UTextBlock::UTextBlock` 0x2bad6e0, UWidget / UUserWidget ctors); the
/// colour and padding values are the 4.26 source's (UNCONFIRMED: the ctors were not disassembled)
fn native_defaults(n: &str) -> HashMap<String, V> {
    let white = || lin([1.0; 4]);
    let slate_white = || V::st(&[("SpecifiedColor", lin([1.0; 4])), ("ColorUseRule", V::Int(0))]);
    let mut m: HashMap<String, V> = HashMap::new();
    let widget = !n.ends_with("Slot") && !matches!(n, "Object" | "WidgetAnimation" | "MaterialInstanceDynamic") && !n.starts_with("Mordhau") && n != "World" && n != "GameViewportClient";
    if widget {
        m.insert("RenderOpacity".into(), V::Float(1.0));
        m.insert("bIsEnabled".into(), V::Bool(true));
    }
    match n {
        "Border" => {
            m.insert("BrushColor".into(), white());
            m.insert("ContentColorAndOpacity".into(), white());
            m.insert("Padding".into(), V::st(&[("Left", V::Float(4.0)), ("Top", V::Float(2.0)), ("Right", V::Float(4.0)), ("Bottom", V::Float(2.0))]));
            m.insert("DesiredSizeScale".into(), V::st(&[("X", V::Float(1.0)), ("Y", V::Float(1.0))]));
        }
        "Image" => {
            m.insert("ColorAndOpacity".into(), white());
        }
        "Button" => {
            m.insert("ColorAndOpacity".into(), white());
            m.insert("BackgroundColor".into(), white());
            // UButton::UButton: WidgetStyle = FCoreStyle "Button" (UE 4.26 CoreStyle.cpp; tagged WidgetStyle values are
            // deltas against it, e.g. a brush that only sets ResourceObject is still a Box with Margin 8/32)
            let mg = |l: f64, t: f64, r: f64, b: f64| V::st(&[("Left", V::Float(l)), ("Top", V::Float(t)), ("Right", V::Float(r)), ("Bottom", V::Float(b))]);
            let bx = |img: &str| {
                V::st(&[
                    ("ImageSize", V::st(&[("X", V::Float(32.0)), ("Y", V::Float(32.0))])),
                    ("Margin", mg(0.25, 0.25, 0.25, 0.25)),
                    ("TintColor", slate_white()),
                    ("DrawAs", V::Int(1)),
                    ("ResourceName", V::Str(format!("../../../Engine/Content/Slate/Common/{img}.png"))),
                ])
            };
            m.insert(
                "WidgetStyle".into(),
                V::st(&[
                    ("Normal", bx("Button")),
                    ("Hovered", bx("Button_Hovered")),
                    ("Pressed", bx("Button_Pressed")),
                    ("Disabled", bx("Button_Disabled")),
                    ("NormalPadding", mg(2.0, 2.0, 2.0, 2.0)),
                    ("PressedPadding", mg(2.0, 3.0, 2.0, 1.0)),
                ]),
            );
        }
        "ComboBoxText" | "ComboBoxString" => {
            // `UComboBoxText::UComboBoxText` rva 0x144ecd0 copies SComboBox's default FArguments style, FCoreStyle
            // "ComboBox" (UE 4.26 CoreStyle.cpp: ComboButtonStyle.ButtonStyle = the "Button" style, Box brushes
            // Common/Button*.png 32x32 Margin 8/32). Without it the BP's tint-only Normal / Hovered deltas drew as
            // stretched images (the flat washed-out hovered box)
            let mg = |l: f64| V::st(&[("Left", V::Float(l)), ("Top", V::Float(l)), ("Right", V::Float(l)), ("Bottom", V::Float(l))]);
            let bx = |img: &str| {
                V::st(&[
                    ("ImageSize", V::st(&[("X", V::Float(32.0)), ("Y", V::Float(32.0))])),
                    ("Margin", mg(0.25)),
                    ("TintColor", slate_white()),
                    ("DrawAs", V::Int(1)),
                    ("ResourceName", V::Str(format!("../../../Engine/Content/Slate/Common/{img}.png"))),
                ])
            };
            let bs = V::st(&[("Normal", bx("Button")), ("Hovered", bx("Button_Hovered")), ("Pressed", bx("Button_Pressed")), ("Disabled", bx("Button_Disabled")), ("NormalPadding", mg(2.0)), ("PressedPadding", mg(2.0))]);
            m.insert("WidgetStyle".into(), V::st(&[("ComboButtonStyle", V::st(&[("ButtonStyle", bs)]))]));
        }
        "ProgressBar" => {
            m.insert("FillColorAndOpacity".into(), white());
        }
        "TextBlock" | "RichTextBlock" => {
            m.insert("ColorAndOpacity".into(), slate_white());
            m.insert("ShadowOffset".into(), V::st(&[("X", V::Float(1.0)), ("Y", V::Float(1.0))]));
        }
        "UserWidget" => {
            m.insert("ColorAndOpacity".into(), white());
            m.insert("ForegroundColor".into(), slate_white());
        }
        _ => {}
    }
    m
}

/// An unported native never no-ops silently (brief 23:36 addendum): each (native, calling Blueprint function) pair is
/// logged once per process as a warning and appended to a TSV the orchestrator reads: `$MH_UI_MISSING_LOG`, else
/// `state/ui_missing_natives.tsv` when the working directory has a `state/` folder (the repo root). Columns: unix time,
/// native (Class::Function), caller (BlueprintClass::Function), process.
pub fn missing_log(native: &str, caller: &str) {
    static SEEN: std::sync::Mutex<Option<std::collections::HashSet<(String, String)>>> = std::sync::Mutex::new(None);
    if !SEEN.lock().map(|mut g| g.get_or_insert_with(Default::default).insert((native.to_string(), caller.to_string()))).unwrap_or(true) {
        return;
    }
    bevy::log::warn!("mh-ui: unported native {native} (called from {caller})");
    let path = match std::env::var_os("MH_UI_MISSING_LOG") {
        Some(p) => std::path::PathBuf::from(p),
        None if std::path::Path::new("state").is_dir() => std::path::PathBuf::from("state/ui_missing_natives.tsv"),
        None => return,
    };
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let exe = std::env::current_exe().ok().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned())).unwrap_or_default();
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{t}\t{native}\t{caller}\t{exe}");
    }
}

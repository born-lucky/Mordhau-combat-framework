//! The Blueprint VM's values and objects: a name-keyed model of UObjects (widgets, slots, the HUD actor, proxies for the
//! game objects the widgets read) and Blueprint values, built from the packages' tagged properties (mh-pak export JSON)
//! and changed by the scripts (vm.rs) and natives (natives.rs). Slate layout (slate.rs) reads the same properties.
//!
//! Enum values: native UMG/Slate enums are stored as their integer value (the order of the UE 4.26 enum declarations,
//! UNCONFIRMED against the exe's UEnum tables, which live in the reflection data not in .rdata names); Blueprint enum
//! values "E_X::NewEnumeratorN" as N (UserDefinedEnum default naming, UNCONFIRMED: an enum whose entries were reordered
//! keeps its old N).

use crate::kismet::ObjRef;
use serde_json::Value;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

pub type Id = u32;

#[derive(Clone, Debug, Default)]
pub enum V {
    #[default]
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Name(String),
    Str(String),
    Text(String),
    Obj(Id),
    /// a package object: texture, material, font, sound, class, enum, ...
    Asset(std::sync::Arc<ObjRef>),
    Struct(Box<BTreeMap<String, V>>),
    Array(Vec<V>),
    Map(Vec<(V, V)>),
    Delegate(Id, String),
    Multi(Vec<(Id, String)>),
}

impl V {
    pub fn truthy(&self) -> bool {
        match self {
            V::None => false,
            V::Bool(b) => *b,
            V::Int(i) => *i != 0,
            V::Float(f) => *f != 0.0,
            V::Name(s) | V::Str(s) | V::Text(s) => !s.is_empty() && s != "None",
            V::Array(a) => !a.is_empty(),
            _ => true,
        }
    }
    pub fn f(&self) -> f64 {
        match self {
            V::Bool(b) => *b as i64 as f64,
            V::Int(i) => *i as f64,
            V::Float(f) => *f,
            V::Str(s) | V::Text(s) => s.trim().parse().unwrap_or(0.0),
            _ => 0.0,
        }
    }
    pub fn i(&self) -> i64 {
        match self {
            V::Bool(b) => *b as i64,
            V::Int(i) => *i,
            V::Float(f) => *f as i64,
            V::Str(s) | V::Text(s) => s.trim().parse().unwrap_or(0),
            V::Name(s) => enum_value(s).unwrap_or(0),
            _ => 0,
        }
    }
    pub fn s(&self) -> String {
        match self {
            V::None => String::new(),
            V::Bool(b) => if *b { "true" } else { "false" }.into(),
            V::Int(i) => i.to_string(),
            V::Float(f) => format!("{f:.6}"),
            V::Name(s) | V::Str(s) | V::Text(s) => s.clone(),
            V::Asset(a) => a.name.clone(),
            _ => String::new(),
        }
    }
    pub fn obj(&self) -> Option<Id> {
        match self {
            V::Obj(i) => Some(*i),
            _ => None,
        }
    }
    pub fn field(&self, k: &str) -> &V {
        static NONE: V = V::None;
        match self {
            V::Struct(m) => m.get(k).unwrap_or(&NONE),
            _ => &NONE,
        }
    }
    pub fn arr(&self) -> &[V] {
        match self {
            V::Array(a) => a,
            _ => &[],
        }
    }
    /// a struct value from (field, value) pairs
    pub fn st(fields: &[(&str, V)]) -> V {
        V::Struct(Box::new(fields.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()))
    }
    pub fn same(&self, o: &V) -> bool {
        match (self, o) {
            (V::None, V::None) => true,
            (V::Obj(a), V::Obj(b)) => a == b,
            (V::Asset(a), V::Asset(b)) => a.name == b.name && a.package == b.package,
            (V::None, V::Obj(_)) | (V::Obj(_), V::None) | (V::None, V::Asset(_)) | (V::Asset(_), V::None) => false,
            (V::Name(a), V::Name(b)) | (V::Str(a), V::Str(b)) | (V::Text(a), V::Text(b)) => a == b,
            (V::Struct(a), V::Struct(b)) => a.len() == b.len() && a.iter().all(|(k, v)| b.get(k).is_some_and(|w| v.same(w))),
            (V::Array(a), V::Array(b)) => a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.same(y)),
            (V::Float(_), _) | (_, V::Float(_)) => self.f() == o.f(),
            _ => self.i() == o.i(),
        }
    }
}


/// FLinearColor / FSlateColor value -> linear RGBA (absent = `d`)
pub fn color_of(v: &V, d: [f64; 4]) -> [f64; 4] {
    let c = match v.field("SpecifiedColor") {
        V::None => v,
        s => s,
    };
    match c {
        V::Struct(m) if m.contains_key("R") || m.contains_key("G") => [c.field("R").f(), c.field("G").f(), c.field("B").f(), match c.field("A") {
            V::None => 1.0,
            a => a.f(),
        }],
        _ => d,
    }
}

pub fn lin(c: [f64; 4]) -> V {
    V::st(&[("R", V::Float(c[0])), ("G", V::Float(c[1])), ("B", V::Float(c[2])), ("A", V::Float(c[3]))])
}

/// FMargin value (Left, Top, Right, Bottom; absent = 0)
pub fn margin_of(v: &V) -> [f64; 4] {
    [v.field("Left").f(), v.field("Top").f(), v.field("Right").f(), v.field("Bottom").f()]
}

pub fn v2_of(v: &V, d: [f64; 2]) -> [f64; 2] {
    match v {
        V::Struct(_) => [match v.field("X") {
            V::None => d[0],
            x => x.f(),
        }, match v.field("Y") {
            V::None => d[1],
            y => y.f(),
        }],
        _ => d,
    }
}

/// Native enum values by "Enum::Entry" / bare entry (UE 4.26 declaration order, UNCONFIRMED: see module doc)
pub fn enum_value(s: &str) -> Option<i64> {
    let (e, k) = s.split_once("::").unwrap_or(("", s));
    let t: &[&str] = match e {
        "ESlateVisibility" => &["Visible", "Collapsed", "Hidden", "HitTestInvisible", "SelfHitTestInvisible"],
        // Mordhau customization enums (rust-armory): EWearableSlot from the PDB (extract/native/types/EWearableSlot.h);
        // EEquipmentCategory / EItemRarity in the order of their UEnum reflection name tables in the exe (.rdata strings
        // at file offsets 0x43d8e20 / 0x43d7d70; values 0.. in that order: UNCONFIRMED, UHT's default numbering)
        "EWearableSlot" => &["Head", "Coif", "UpperChest", "LowerChest", "Shoulders", "Arms", "Hands", "Legs", "Feet", "Total", "Invalid"],
        "EEquipmentCategory" => &["Undefined", "OneHanded", "TwoHanded", "Ranged", "Shield", "Utility"],
        "EItemRarity" => &["Common", "Uncommon", "Rare", "Epic", "Legendary", "Exclusive"],
        "ESlateSizeRule" => &["Automatic", "Fill"],
        "ETextJustify" => &["Left", "Center", "Right"],
        "ESlateBrushDrawType" => &["NoDrawType", "Box", "Border", "Image"],
        "ESlateBrushTileType" => &["NoTile", "Horizontal", "Vertical", "Both"],
        "ESlateBrushMirrorType" => &["NoMirror", "Horizontal", "Vertical", "Both"],
        "EStretch" => &["None", "Fill", "ScaleToFit", "ScaleToFitX", "ScaleToFitY", "ScaleToFill", "ScaleBySafeZone", "UserSpecified"],
        "EStretchDirection" => &["Both", "DownOnly", "UpOnly"],
        "EOrientation" => &["Orient_Horizontal", "Orient_Vertical"],
        "EProgressBarFillType" => &["LeftToRight", "RightToLeft", "FillFromCenter", "TopToBottom", "BottomToTop"],
        "ESlateColorStylingMode" => &["UseColor_Specified", "UseColor_Specified_Link", "UseColor_Foreground", "UseColor_Foreground_Subdued", "UseColor_UseStyle"],
        "EWidgetClipping" => &["Inherit", "ClipToBounds", "ClipToBoundsWithoutIntersecting", "ClipToBoundsAlways", "OnDemand"],
        "ETextWrappingPolicy" => &["DefaultWrapping", "AllowPerCharacterWrapping"],
        "ECheckBoxState" => &["Unchecked", "Checked", "Undetermined"],
        "EButtonClickMethod" => &["DownAndUp", "MouseDown", "MouseUp", "PreciseClick"],
        "EUMGSequencePlayMode" => &["Forward", "Reverse", "PingPong"],
        "EWidgetSpace" => &["World", "Screen"],
        "EVirtualKeyboardType" => &["Default", "Number", "Web", "Email", "Password", "AlphaNumeric"],
        "ETextCommit" => &["Default", "OnEnter", "OnUserMovedFocus", "OnCleared"],
        "ESelectInfo" => &["OnKeyPress", "OnNavigation", "OnMouseClick", "Direct"],
        "EDescendantScrollDestination" => &["IntoView", "TopOrLeft", "Center", "BottomOrRight"],
        "EScrollWhenFocusChanges" => &["NoScroll", "InstantScroll", "AnimatedScroll"],
        "ESlateDrawEffect" => &["None", "NoBlending", "PreMultipliedAlpha", "NoGamma", "InvertAlpha"],
        "EFlowDirectionPreference" => &["Inherit", "Culture", "LeftToRight", "RightToLeft"],
        _ => &[],
    };
    if let Some(i) = t.iter().position(|x| *x == k) {
        return Some(i as i64);
    }
    // EHorizontalAlignment / EVerticalAlignment are stored without the enum prefix ("HAlign_Center")
    for (p, names) in [("HAlign_", ["Fill", "Left", "Center", "Right"]), ("VAlign_", ["Fill", "Top", "Center", "Bottom"])] {
        if let Some(r) = k.strip_prefix(p) {
            return names.iter().position(|x| *x == r).map(|i| i as i64);
        }
    }
    if let Some(r) = k.strip_prefix("NewEnumerator") {
        return r.parse().ok();
    }
    None
}

/// mh-pak export JSON property -> value. `local` resolves an ObjectPath into the same package (slots, contents,
/// widgets) to a live object id when the instantiation has one.
pub fn json_v(v: &Value, local: &dyn Fn(&str) -> Option<Id>) -> V {
    match v {
        Value::Null => V::None,
        Value::Bool(b) => V::Bool(*b),
        Value::Number(n) => {
            if n.is_f64() {
                V::Float(n.as_f64().unwrap_or(0.0))
            } else {
                V::Int(n.as_i64().unwrap_or(0))
            }
        }
        Value::String(s) => {
            if s.contains("::") || s.starts_with("HAlign_") || s.starts_with("VAlign_") {
                if let Some(i) = enum_value(s) {
                    return V::Int(i);
                }
            }
            V::Name(s.clone())
        }
        // a TMap property is exported as [{"Key": k, "Value": v}, ...]
        Value::Array(a) if !a.is_empty() && a.iter().all(|x| x.as_object().is_some_and(|o| o.len() == 2 && o.contains_key("Key") && o.contains_key("Value"))) => {
            V::Map(a.iter().map(|x| (json_v(&x["Key"], local), json_v(&x["Value"], local))).collect())
        }
        Value::Array(a) => V::Array(a.iter().map(|x| json_v(x, local)).collect()),
        Value::Object(o) => {
            if let Some(op) = o.get("ObjectPath").and_then(Value::as_str) {
                if let Some(id) = local(op) {
                    return V::Obj(id);
                }
                let on = o.get("ObjectName").and_then(Value::as_str).unwrap_or("");
                let (cls, rest) = on.split_once('\'').unwrap_or(("", on));
                let name = rest.trim_end_matches('\'');
                let name = name.rsplit([':', '.']).next().unwrap_or(name);
                let pkg = mh_pak::reader::strip_index(op);
                return V::Asset(std::sync::Arc::new(ObjRef { package: crate::kismet::content_path(pkg), name: name.to_string(), class: cls.to_string(), outer: String::new() }));
            }
            if o.contains_key("SourceString") || o.contains_key("CultureInvariantString") || o.contains_key("LocalizedString") || (o.contains_key("Key") && o.contains_key("Namespace")) {
                return V::Text(mh_assets::umg::text(Some(v)));
            }
            if o.contains_key("KeyName") && o.len() == 1 {
                return V::st(&[("KeyName", V::Name(o["KeyName"].as_str().unwrap_or("").into()))]);
            }
            V::Struct(Box::new(o.iter().filter(|(k, _)| k.as_str() != "Hex").map(|(k, x)| (k.clone(), json_v(x, local))).collect()))
        }
    }
}

/// A class: Blueprint generated class (functions, members, CDO, widget tree) or a native class (name chain only)
#[derive(Default)]
pub struct Class {
    pub name: String,
    /// the Blueprint package, "" for native classes
    pub package: String,
    pub parent: Option<Rc<Class>>,
    pub funcs: HashMap<String, Rc<crate::kismet::Func>>,
    pub members: Vec<crate::kismet::Prop>,
    /// class default object properties (own, already merged over the parent's)
    pub cdo: HashMap<String, V>,
    /// the package's exports when it has a WidgetTree (instantiated per object)
    pub exports: Option<Rc<Vec<Value>>>,
    pub tree_root: Option<usize>,
    /// UWidgetBlueprintGeneratedClass Bindings: (widget, property, function or "", source property path)
    pub bindings: Vec<(String, String, String, Vec<String>)>,
    /// ComponentDelegateBindings: (component property, delegate, function)
    pub comp_delegates: Vec<(String, String, String)>,
    /// widget animations: name -> length in seconds (MovieScene PlaybackRange / TickResolution)
    pub anims: HashMap<String, f64>,
    /// widget animations: name -> property tracks (anim.rs)
    pub anim_data: HashMap<String, Rc<crate::anim::Anim>>,
}

impl Class {
    /// class names from this one up (Blueprint chain, then the native chain of the first native ancestor)
    pub fn chain(&self) -> Vec<String> {
        let mut out = vec![];
        let mut c = Some(self);
        while let Some(k) = c {
            if k.package.is_empty() {
                out.extend(native_chain(&k.name).iter().map(|s| s.to_string()));
                break;
            }
            out.push(k.name.clone());
            c = k.parent.as_deref();
        }
        out
    }
    pub fn isa(&self, n: &str) -> bool {
        self.chain().iter().any(|c| c == n)
    }
    /// the first native ancestor's name (the widget kind Slate lays out)
    pub fn native(&self) -> &str {
        let mut c = self;
        while !c.package.is_empty() {
            match &c.parent {
                Some(p) => c = p,
                None => return "Object",
            }
        }
        &c.name
    }
    pub fn func(&self, n: &str) -> Option<Rc<crate::kismet::Func>> {
        let mut c = Some(self);
        while let Some(k) = c {
            if let Some(f) = k.funcs.get(n) {
                return Some(f.clone());
            }
            c = k.parent.as_deref();
        }
        None
    }
    pub fn tree_class(&self) -> Option<&Class> {
        let mut c = Some(self);
        while let Some(k) = c {
            if k.tree_root.is_some() {
                return Some(k);
            }
            c = k.parent.as_deref();
        }
        None
    }
}

/// Native class ancestry (UE 4.26 class declarations: UMG Components/*.h, Slate; Mordhau classes from the PDB type
/// names). Unknown classes are their own chain + Object.
pub fn native_chain(n: &str) -> Vec<&str> {
    let w = ["Widget", "Visual", "Object"];
    let content = ["ContentWidget", "PanelWidget", "Widget", "Visual", "Object"];
    let panel = ["PanelWidget", "Widget", "Visual", "Object"];
    let slot = ["PanelSlot", "Visual", "Object"];
    let mut v: Vec<&str> = vec![];
    let leaked: &'static str = Box::leak(n.to_string().into_boxed_str());
    v.push(leaked);
    let tail: &[&str] = match n {
        "TextBlock" | "RichTextBlock" | "MultiLineEditableText" | "MultiLineEditableTextBox" => &["TextLayoutWidget", "Widget", "Visual", "Object"],
        "Border" | "Button" | "SizeBox" | "ScaleBox" | "BackgroundBlur" | "InvalidationBox" | "RetainerBox" | "SafeZone" | "NamedSlot" | "CheckBox" | "MenuAnchor" | "ComboButton" => &content,
        "CanvasPanel" | "Overlay" | "HorizontalBox" | "VerticalBox" | "WidgetSwitcher" | "ScrollBox" | "UniformGridPanel" | "GridPanel" | "WrapBox" | "WindowTitleBarArea" => &panel,
        "ListView" | "TileView" | "TreeView" => &["ListViewBase", "TableViewBase", "Widget", "Visual", "Object"],
        "UserWidget" => &w,
        "MordhauHUD" => &["HUD", "Actor", "Object"],
        "MordhauCharacter" => &["AdvancedCharacter", "Character", "Pawn", "Actor", "Object"],
        "MordhauWeapon" => &["MordhauEquipment", "Actor", "Object"],
        "MordhauPlayerController" => &["AdvancedPlayerController", "PlayerController", "Controller", "Actor", "Object"],
        "MordhauPlayerState" => &["PlayerState", "Info", "Actor", "Object"],
        "MordhauGameState" => &["GameState", "GameStateBase", "Info", "Actor", "Object"],
        _ if n.ends_with("Slot") && n != "Slot" => &slot,
        "Object" => &[],
        _ if n.starts_with('U') || n.contains("Widget") => &w,
        _ => &["Object"],
    };
    v.extend_from_slice(tail);
    v
}

/// A UObject instance
#[derive(Default)]
pub struct Obj {
    pub name: String,
    pub class: Rc<Class>,
    pub props: HashMap<String, V>,
    /// ubergraph persistent frame (EX_LetValueOnPersistentFrame, ExecuteUbergraph locals)
    pub uber: Rc<RefCell<HashMap<String, V>>>,
    /// panel widgets: their slot objects in order
    pub slots: Vec<Id>,
    /// slot objects: the widget they hold; widgets: the slot holding them
    pub content: Option<Id>,
    pub parent_slot: Option<Id>,
    /// the slot's panel
    pub panel: Option<Id>,
    /// user widgets: the instantiated tree's root widget
    pub root: Option<Id>,
    /// the user widget whose tree this widget belongs to (binding / delegate owner)
    pub owner: Option<Id>,
    pub constructed: bool,
    pub alive: bool,
}

impl Obj {
    pub fn get(&self, k: &str) -> &V {
        static NONE: V = V::None;
        self.props.get(k).or_else(|| self.class.cdo.get(k)).unwrap_or(&NONE)
    }
}

/// Zero value of a property type (FProperty::InitializeValue: zeroed memory; a struct's fields default per field,
/// read as absent = zero here)
pub fn zero(p: &crate::kismet::Prop) -> V {
    match p.ty.as_str() {
        "BoolProperty" => V::Bool(false),
        "IntProperty" | "Int64Property" | "ByteProperty" | "EnumProperty" | "Int16Property" | "Int8Property" | "UInt16Property" | "UInt32Property" | "UInt64Property" => V::Int(0),
        "FloatProperty" | "DoubleProperty" => V::Float(0.0),
        "StrProperty" => V::Str(String::new()),
        "NameProperty" => V::Name("None".into()),
        "TextProperty" => V::Text(String::new()),
        "ArrayProperty" | "SetProperty" => V::Array(vec![]),
        "MapProperty" => V::Map(vec![]),
        "StructProperty" => V::Struct(Box::default()),
        "MulticastDelegateProperty" | "MulticastInlineDelegateProperty" | "MulticastSparseDelegateProperty" => V::Multi(vec![]),
        _ => V::None,
    }
}

/// a tagged struct property holds only the members that differ from the archetype's value (UE delta serialization of
/// struct properties: UStruct::SerializeTaggedProperties against the defaults); the full value is the archetype's
/// struct with these members laid over it, recursively
pub fn merge_over(base: &V, over: V) -> V {
    match (base, over) {
        (V::Struct(b), V::Struct(o)) => {
            let mut m = b.clone();
            for (k, v) in o.into_iter() {
                let nv = match m.get(&k) {
                    Some(bv) => merge_over(bv, v),
                    None => v,
                };
                m.insert(k, nv);
            }
            V::Struct(m)
        }
        (_, o) => o,
    }
}

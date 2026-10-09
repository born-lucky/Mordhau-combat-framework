//! Native functions the UI Blueprints call: UMG widget methods, the Kismet libraries, GameplayStatics, the Mordhau C++
//! API the menus use. Each is the UE 4.26 behaviour of the named UFUNCTION (UMG Components/*.cpp, Kismet*Library.cpp,
//! GameplayStatics.cpp) restricted to what the widget model needs; the exe's thunks are the `exec<Name>` functions
//! (e.g. `UWidgetSwitcher::SetActiveWidgetIndex` 0x2c18010, `UWidgetBlueprintLibrary::Create` 0x2bccb80,
//! `UUserWidget::AddToViewport` 0x2bbf1f0). Mordhau's own natives (UMordhau*Library, AMordhauHUD, settings) read the
//! host's proxy objects (host.rs); anything not implemented returns None and is counted in Vm::missing (UNCONFIRMED).

use crate::kismet::ObjRef;
use crate::model::*;
use crate::vm::{Action, Ctx, Latent, Playing, Vm};

pub struct NRet {
    pub ret: V,
    /// (argument index, value) written back to out / by-ref parameters
    pub outs: Vec<(usize, V)>,
}

const TICKS_PER_SEC: i64 = 10_000_000;
fn timespan(t: i64) -> V {
    V::st(&[("Ticks", V::Int(t))])
}
fn ticks(v: &V) -> i64 {
    v.field("Ticks").i()
}

fn r(v: V) -> Option<NRet> {
    Some(NRet { ret: v, outs: vec![] })
}
fn outs(ret: V, o: Vec<(usize, V)>) -> Option<NRet> {
    Some(NRet { ret, outs: o })
}

/// UKismetTextLibrary::Conv_IntToText with grouping (FText::AsNumber, invariant culture: ',' every 3 digits)
fn group(i: i64) -> String {
    let s = i.abs().to_string();
    let mut out = String::new();
    for (k, c) in s.chars().enumerate() {
        if k > 0 && (s.len() - k) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    if i < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// FText::AsNumber for floats (default FNumberFormattingOptions: max 3 fractional digits, grouping)
fn float_text(f: f64, min_frac: i64, max_frac: i64) -> String {
    let max = max_frac.clamp(0, 10) as usize;
    let mut s = format!("{:.*}", max, f);
    if s.contains('.') {
        let min = min_frac.clamp(0, 10) as usize;
        let (a, b) = s.split_once('.').map(|(a, b)| (a.to_string(), b.to_string())).unwrap();
        let mut b = b;
        while b.len() > min && b.ends_with('0') {
            b.pop();
        }
        s = if b.is_empty() { a } else { format!("{a}.{b}") };
    }
    s
}

fn vis_visible(v: i64) -> bool {
    // ESlateVisibility: Visible 0, Collapsed 1, Hidden 2, HitTestInvisible 3, SelfHitTestInvisible 4
    !(v == 1 || v == 2)
}

pub fn world(vm: &Vm, k: &str) -> V {
    vm.world.get(k).map(|&i| V::Obj(i)).unwrap_or_default()
}

/// FText::Format with named ({Name}) and ordered ({0}) arguments
fn format_text(pat: &str, args: &[V]) -> String {
    let mut out = pat.to_string();
    for (i, a) in args.iter().enumerate() {
        let name = a.field("ArgumentName").s();
        let ty = a.field("ArgumentValueType").i();
        let val = match ty {
            0 | 1 => group(a.field("ArgumentValueInt").i()),
            2 | 3 => float_text(a.field("ArgumentValueFloat").f(), 0, 3),
            _ => a.field("ArgumentValue").s(),
        };
        out = out.replace(&format!("{{{name}}}"), &val).replace(&format!("{{{i}}}"), &val);
    }
    out
}

/// The slot holding a widget, if it is of `kind` (UWidgetLayoutLibrary::SlotAs*)
fn slot_as(vm: &Vm, w: &V, kind: &str) -> V {
    let Some(id) = w.obj() else { return V::None };
    match vm.o(id).parent_slot {
        Some(s) if vm.o(s).class.name == kind => V::Obj(s),
        _ => V::None,
    }
}

/// Add `child` to panel `p` with a new slot of the panel's slot class (UPanelWidget::AddChild)
pub fn add_child(vm: &mut Vm, p: crate::model::Id, child: crate::model::Id) -> V {
    if !vm.alive(p) || !vm.alive(child) {
        return V::None;
    }
    remove_from_parent(vm, child);
    let pk = vm.o(p).class.native().to_string();
    let sc = match pk.as_str() {
        "CanvasPanel" => "CanvasPanelSlot",
        "Overlay" => "OverlaySlot",
        "HorizontalBox" => "HorizontalBoxSlot",
        "VerticalBox" => "VerticalBoxSlot",
        "WidgetSwitcher" => "WidgetSwitcherSlot",
        "ScrollBox" => "ScrollBoxSlot",
        "UniformGridPanel" => "UniformGridSlot",
        "GridPanel" => "GridSlot",
        "WrapBox" => "WrapBoxSlot",
        "SizeBox" => "SizeBoxSlot",
        "Border" => "BorderSlot",
        "Button" => "ButtonSlot",
        "ScaleBox" => "ScaleBoxSlot",
        "BackgroundBlur" => "BackgroundBlurSlot",
        "SafeZone" => "SafeZoneSlot",
        _ => "PanelSlot",
    };
    let cls = vm.native_class(sc);
    let s = vm.new_obj(cls, sc);
    vm.om(s).content = Some(child);
    vm.om(s).panel = Some(p);
    vm.set(s, "Content", V::Obj(child));
    vm.set(s, "Parent", V::Obj(p));
    vm.om(child).parent_slot = Some(s);
    vm.set(child, "Slot", V::Obj(s));
    // single-content widgets replace their content
    if matches!(sc, "SizeBoxSlot" | "BorderSlot" | "ButtonSlot" | "ScaleBoxSlot" | "BackgroundBlurSlot" | "SafeZoneSlot") {
        vm.om(p).slots.clear();
    }
    vm.om(p).slots.push(s);
    let constructed = vm.o(p).constructed || vm.viewport.iter().any(|x| x.0 == p) || ancestor_constructed(vm, p);
    if constructed && vm.o(child).class.tree_class().is_some() {
        vm.construct(child);
    }
    V::Obj(s)
}

fn ancestor_constructed(vm: &Vm, mut w: crate::model::Id) -> bool {
    for _ in 0..256 {
        if let Some(ow) = vm.o(w).owner {
            if vm.o(ow).constructed {
                return true;
            }
        }
        match vm.o(w).parent_slot.and_then(|s| vm.o(s).panel) {
            Some(p) => w = p,
            None => return vm.o(w).constructed,
        }
    }
    false
}

pub fn remove_from_parent(vm: &mut Vm, w: crate::model::Id) {
    vm.viewport.retain(|x| x.0 != w);
    if let Some(s) = vm.o(w).parent_slot {
        if let Some(p) = vm.o(s).panel {
            vm.om(p).slots.retain(|&x| x != s);
        }
        vm.om(s).content = None;
    }
    vm.om(w).parent_slot = None;
}

fn children(vm: &Vm, p: crate::model::Id) -> Vec<crate::model::Id> {
    vm.o(p).slots.iter().filter_map(|&s| vm.o(s).content).collect()
}

fn anim_name(v: &V) -> String {
    match v {
        V::Asset(a) => a.name.trim_end_matches("_INST").to_string(),
        _ => String::new(),
    }
}

fn play(vm: &mut Vm, w: crate::model::Id, anim: &V, start: f64, loops: i64, reverse: bool, speed: f64) {
    let name = anim_name(anim);
    let len = vm.o(w).class.tree_class().and_then(|c| c.anims.get(&name).copied()).unwrap_or(0.0);
    let dur = ((len - start).max(0.0) / speed.max(1e-3)) * loops.max(1) as f64;
    vm.playing.retain(|p| !(p.widget == w && p.anim == name));
    vm.playing.push(Playing { widget: w, anim: name, start: vm.time, end: vm.time + dur, reverse, loops: if loops == 0 { 0 } else { 1 }, offset: start, speed: speed.max(1e-3), len });
    vm.apply_animations();
}

/// Call a native: `ctx` is the target object (method) or Static; `class` the UFunction's owner class when known
pub fn call(vm: &mut Vm, ctx: Ctx, class: &str, name: &str, a: &[V]) -> Option<NRet> {
    if let Some(r) = crate::armory_natives::call(vm, ctx, class, name, a) { return Some(r) } // rust-armory: Armory / Mercenaries natives
    let t = match ctx {
        Ctx::Obj(i) => i,
        _ => 0,
    };
    let a0 = a.first().cloned().unwrap_or_default();
    let a1 = a.get(1).cloned().unwrap_or_default();
    let a2 = a.get(2).cloned().unwrap_or_default();
    let ai = |k: usize| a.get(k).map(V::i).unwrap_or(0);
    let af = |k: usize| a.get(k).map(V::f).unwrap_or(0.0);
    let ab = |k: usize| a.get(k).map(V::truthy).unwrap_or(false);
    let as_ = |k: usize| a.get(k).map(V::s).unwrap_or_default();

    // ---- Kismet math (UKismetMathLibrary), by name pattern -------------------------------------------------------
    if let Some((op, ty)) = name.split_once('_') {
        let num = |x: &V, ty: &str| -> V {
            if ty.starts_with("Float") || ty.starts_with("Double") {
                V::Float(x.f())
            } else {
                V::Int(x.i())
            }
        };
        let ty2 = ty;
        let isf = ty2.contains("Float") || ty2.contains("Double");
        let bin = |f: fn(f64, f64) -> f64, g: fn(i64, i64) -> i64| -> V {
            if isf {
                V::Float(f(a0.f(), a1.f()))
            } else {
                V::Int(g(a0.i(), a1.i()))
            }
        };
        let scalar_ty = matches!(ty2, "IntInt" | "FloatFloat" | "ByteByte" | "Int64Int64" | "IntFloat" | "FloatInt" | "DoubleDouble");
        if scalar_ty {
            let _ = num;
            match op {
                "Add" => return r(bin(|x, y| x + y, |x, y| x.wrapping_add(y))),
                "Subtract" => return r(bin(|x, y| x - y, |x, y| x.wrapping_sub(y))),
                "Multiply" => return r(bin(|x, y| x * y, |x, y| x.wrapping_mul(y))),
                // UKismetMathLibrary::Divide_IntInt / Divide_FloatFloat: division by zero logs and returns 0
                "Divide" => return r(bin(|x, y| if y == 0.0 { 0.0 } else { x / y }, |x, y| if y == 0 { 0 } else { x / y })),
                "Percent" => return r(bin(|x, y| if y == 0.0 { 0.0 } else { x % y }, |x, y| if y == 0 { 0 } else { x % y })),
                "Less" => return r(V::Bool(a0.f() < a1.f())),
                "Greater" => return r(V::Bool(a0.f() > a1.f())),
                "LessEqual" => return r(V::Bool(a0.f() <= a1.f())),
                "GreaterEqual" => return r(V::Bool(a0.f() >= a1.f())),
                "EqualEqual" => return r(V::Bool(a0.f() == a1.f())),
                "NotEqual" => return r(V::Bool(a0.f() != a1.f())),
                _ => {}
            }
        }
        match (op, ty) {
            ("EqualEqual", "ObjectObject" | "ClassClass" | "NameName" | "StrStr" | "TextText" | "KeyKey" | "BoolBool") => {
                return r(V::Bool(match ty {
                    "KeyKey" => key_name(&a0) == key_name(&a1),
                    "StrStr" | "TextText" => a0.s().eq_ignore_ascii_case(&a1.s()) || a0.s() == a1.s(),
                    _ => a0.same(&a1),
                }))
            }
            ("NotEqual", "ObjectObject" | "ClassClass" | "NameName" | "StrStr" | "TextText" | "KeyKey" | "BoolBool") => {
                return r(V::Bool(match ty {
                    "KeyKey" => key_name(&a0) != key_name(&a1),
                    "StrStr" | "TextText" => !a0.s().eq_ignore_ascii_case(&a1.s()),
                    _ => !a0.same(&a1),
                }))
            }
            ("EqualEqual", "StriStri") => return r(V::Bool(a0.s().eq_ignore_ascii_case(&a1.s()))),
            ("NotEqual", "StriStri") => return r(V::Bool(!a0.s().eq_ignore_ascii_case(&a1.s()))),
            ("Not", "PreBool") => return r(V::Bool(!a0.truthy())),
            ("Add", "Vector2DVector2D") => return r(V::st(&[("X", V::Float(a0.field("X").f() + a1.field("X").f())), ("Y", V::Float(a0.field("Y").f() + a1.field("Y").f()))])),
            ("Subtract", "Vector2DVector2D") => return r(V::st(&[("X", V::Float(a0.field("X").f() - a1.field("X").f())), ("Y", V::Float(a0.field("Y").f() - a1.field("Y").f()))])),
            ("Multiply", "Vector2DFloat") => return r(V::st(&[("X", V::Float(a0.field("X").f() * a1.f())), ("Y", V::Float(a0.field("Y").f() * a1.f()))])),
            ("Divide", "Vector2DFloat") => {
                let d = if a1.f() == 0.0 { 1.0 } else { a1.f() };
                return r(V::st(&[("X", V::Float(a0.field("X").f() / d)), ("Y", V::Float(a0.field("Y").f() / d))]));
            }
            ("Multiply", "LinearColorFloat") => {
                let c = color_of(&a0, [0.0; 4]);
                let k = a1.f();
                return r(lin([c[0] * k, c[1] * k, c[2] * k, c[3] * k]));
            }
            _ => {}
        }
        if op == "Conv" && (ty.starts_with("SoftObject") || ty.starts_with("SoftClass")) && ty.ends_with("ToString") {
            return r(V::Str(match &a0 {
                V::Struct(_) => a0.field("AssetPathName").s(),
                V::Asset(x) => format!("{}.{}", x.package, x.name),
                v => v.s(),
            }));
        }
        if op == "Conv" {
            let to = ty.split_once("To").map(|x| x.1).unwrap_or("");
            let from = ty.split_once("To").map(|x| x.0).unwrap_or("");
            let v = match to {
                "Float" | "Double" => V::Float(a0.f()),
                "Int" | "Int64" => V::Int(if from == "Float" { a0.f() as i64 } else { a0.i() }),
                "Byte" => V::Int(a0.i() & 0xff),
                "Bool" => V::Bool(a0.truthy()),
                "String" => V::Str(match (from, &a0) {
                    ("Float", x) => format!("{:.6}", x.f()),
                    ("Bool", x) => if x.truthy() { "true".into() } else { "false".into() },
                    ("Object", V::Obj(id)) => vm.o(*id).name.clone(),
                    _ => a0.s(),
                }),
                "Text" => V::Text(match from {
                    "Int" | "Byte" | "Int64" => {
                        // Conv_IntToText(Value, bAlwaysSign, bUseGrouping = true, MinimumIntegralDigits, MaximumIntegralDigits)
                        let g = a.get(2).map(V::truthy).unwrap_or(true);
                        if g { group(a0.i()) } else { a0.i().to_string() }
                    }
                    "Float" => float_text(a0.f(), a.get(5).map(V::i).unwrap_or(0), a.get(6).map(V::i).unwrap_or(3)),
                    "Bool" => if a0.truthy() { "true".into() } else { "false".into() },
                    _ => a0.s(),
                }),
                "Name" => V::Name(a0.s()),
                "Vector2D" => a0.clone(),
                "LinearColor" => match &a0 {
                    V::Struct(_) if a0.field("R").i() > 1 || matches!(a0.field("R"), V::Int(_)) => {
                        // FColor -> FLinearColor (sRGB decode, FLinearColor(FColor) ctor)
                        let s = |x: f64| {
                            let c = x / 255.0;
                            if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
                        };
                        lin([s(a0.field("R").f()), s(a0.field("G").f()), s(a0.field("B").f()), a0.field("A").f() / 255.0])
                    }
                    _ => a0.clone(),
                },
                "SlateColor" => V::st(&[("SpecifiedColor", a0.clone()), ("ColorUseRule", V::Int(0))]),
                _ => a0.clone(),
            };
            return r(v);
        }
    }
    match name {
        // ---- math -------------------------------------------------------------------------------------------------
        "BooleanAND" => r(V::Bool(ab(0) && ab(1))),
        "BooleanOR" => r(V::Bool(ab(0) || ab(1))),
        "BooleanXOR" => r(V::Bool(ab(0) ^ ab(1))),
        "BooleanNAND" => r(V::Bool(!(ab(0) && ab(1)))),
        "FClamp" => r(V::Float(af(0).max(af(1)).min(af(2)))),
        "Clamp" => r(V::Int(ai(0).max(ai(1)).min(ai(2)))),
        "Max" => r(V::Int(ai(0).max(ai(1)))),
        "Min" => r(V::Int(ai(0).min(ai(1)))),
        "FMax" => r(V::Float(af(0).max(af(1)))),
        "FMin" => r(V::Float(af(0).min(af(1)))),
        "Abs" => r(V::Float(af(0).abs())),
        "Abs_Int" => r(V::Int(ai(0).abs())),
        "FTrunc" => r(V::Int(af(0).trunc() as i64)),
        "FFloor" => r(V::Int(af(0).floor() as i64)),
        "FCeil" => r(V::Int(af(0).ceil() as i64)),
        // UKismetMathLibrary::Round = FMath::RoundToInt (round half up: FloorToInt(x + 0.5))
        "Round" => r(V::Int((af(0) + 0.5).floor() as i64)),
        "Sin" => r(V::Float(af(0).sin())),
        "Cos" => r(V::Float(af(0).cos())),
        "Sqrt" => r(V::Float(af(0).max(0.0).sqrt())),
        "MultiplyMultiply_FloatFloat" => r(V::Float(af(0).powf(af(1)))),
        "Lerp" => r(V::Float(af(0) + (af(1) - af(0)) * af(2))),
        "MapRangeClamped" | "MapRangeUnclamped" => {
            let (v, ia, ib, oa, ob) = (af(0), af(1), af(2), af(3), af(4));
            let mut k = if ib == ia { 0.0 } else { (v - ia) / (ib - ia) };
            if name == "MapRangeClamped" {
                k = k.clamp(0.0, 1.0);
            }
            r(V::Float(oa + (ob - oa) * k))
        }
        // FMath::FInterpTo / FInterpConstantTo (UnrealMathUtility.cpp)
        "FInterpTo" => {
            let (cur, tgt, dt, sp) = (af(0), af(1), af(2), af(3));
            if sp <= 0.0 {
                return r(V::Float(tgt));
            }
            let d = tgt - cur;
            if d * d < 1e-8 {
                return r(V::Float(tgt));
            }
            r(V::Float(cur + d * (dt * sp).clamp(0.0, 1.0)))
        }
        "FInterpTo_Constant" => {
            let (cur, tgt, dt, sp) = (af(0), af(1), af(2), af(3));
            let d = tgt - cur;
            if d * d < 1e-8 {
                return r(V::Float(tgt));
            }
            let step = sp * dt;
            r(V::Float(cur + d.clamp(-step, step)))
        }
        "NearlyEqual_FloatFloat" => r(V::Bool((af(0) - af(1)).abs() <= a.get(2).map(V::f).unwrap_or(1e-6))),
        "InRange_FloatFloat" | "InRange_IntInt" => {
            let lo = if ab(3) { af(0) >= af(1) } else { af(0) > af(1) };
            let hi = if ab(4) { af(0) <= af(2) } else { af(0) < af(2) };
            r(V::Bool(lo && hi))
        }
        "SelectFloat" | "SelectInt" | "SelectString" | "SelectText" | "SelectObject" | "SelectColor" | "SelectName" | "SelectVector" | "SelectClass" => r(if ab(2) { a0 } else { a1 }),
        "Select" => {
            // UKismetMathLibrary::Select wildcard (K2Node_Select compiles to a switch; this is the function form)
            r(if ab(2) { a0 } else { a1 })
        }
        "MakeColor" => r(lin([af(0), af(1), af(2), a.get(3).map(V::f).unwrap_or(1.0)])),
        "MakeVector2D" => r(V::st(&[("X", V::Float(af(0))), ("Y", V::Float(af(1)))])),
        "MakeVector" => r(V::st(&[("X", V::Float(af(0))), ("Y", V::Float(af(1))), ("Z", V::Float(af(2)))])),
        "BreakVector2D" => outs(V::None, vec![(1, V::Float(a0.field("X").f())), (2, V::Float(a0.field("Y").f()))]),
        "BreakVector" => outs(V::None, vec![(1, V::Float(a0.field("X").f())), (2, V::Float(a0.field("Y").f())), (3, V::Float(a0.field("Z").f()))]),
        "BreakColor" => {
            let c = color_of(&a0, [0.0; 4]);
            outs(V::None, (0..4).map(|k| (k + 1, V::Float(c[k]))).collect())
        }
        "RGBToHSV" | "HSVToRGB" => r(a0),
        "RandomInteger" => r(V::Int(0)),
        "RandomFloat" => r(V::Float(0.5)),
        // deterministic stand-in for FMath::FRandRange (the midpoint; offscreen evidence must be reproducible: UNCONFIRMED)
        "RandomFloatInRange" => r(V::Float((af(0) + af(1)) * 0.5)),
        // UKismetSystemLibrary::Set*PropertyByName(Object, PropertyName, Value): writes the named property (UE 4.26
        // KismetSystemLibrary.cpp; the generic thunk finds the FProperty by name on the object's class)
        "SetBoolPropertyByName" | "SetBytePropertyByName" | "SetIntPropertyByName" | "SetFloatPropertyByName" | "SetObjectPropertyByName" | "SetStringPropertyByName" | "SetNamePropertyByName" | "SetTextPropertyByName" | "SetClassPropertyByName" => {
            if let Some(o) = a0.obj().filter(|&o| vm.alive(o)) {
                let v = a.get(2).cloned().unwrap_or_default();
                vm.set(o, &as_(1), v);
            }
            r(V::None)
        }
        // UMenuAnchor::Close: the anchored popup goes away (popups from MenuAnchor are not opened by the rewrite yet)
        "Close" if vm.alive(t) && vm.o(t).class.native() == "MenuAnchor" => {
            vm.set(t, "__open", V::Bool(false));
            r(V::None)
        }
        "RandomIntegerInRange" => r(V::Int(ai(0))),
        "MakeLiteralInt" | "MakeLiteralFloat" | "MakeLiteralBool" | "MakeLiteralString" | "MakeLiteralText" | "MakeLiteralName" | "MakeLiteralByte" => r(a0),
        // ---- strings / text ---------------------------------------------------------------------------------------
        "Concat_StrStr" => r(V::Str(as_(0) + &as_(1))),
        "Len" => r(V::Int(as_(0).chars().count() as i64)),
        "GetSubstring" => r(V::Str(as_(0).chars().skip(ai(1).max(0) as usize).take(ai(2).max(0) as usize).collect())),
        "Left" => r(V::Str(as_(0).chars().take(ai(1).max(0) as usize).collect())),
        "Right" => {
            let s: Vec<char> = as_(0).chars().collect();
            let n = (ai(1).max(0) as usize).min(s.len());
            r(V::Str(s[s.len() - n..].iter().collect()))
        }
        "StartsWith" => r(V::Bool(as_(0).to_lowercase().starts_with(&as_(1).to_lowercase()))),
        "EndsWith" => r(V::Bool(as_(0).to_lowercase().ends_with(&as_(1).to_lowercase()))),
        "Contains" => r(V::Bool(as_(0).to_lowercase().contains(&as_(1).to_lowercase()))),
        "Replace" => r(V::Str(as_(0).replace(&as_(1), &as_(2)))),
        "ToUpper" | "TextToUpper" => r(V::Str(as_(0).to_uppercase())),
        "ToLower" | "TextToLower" => r(V::Str(as_(0).to_lowercase())),
        "Trim" | "TrimTrailing" | "TextTrimPrecedingAndTrailing" | "TextTrimPreceding" | "TextTrimTrailing" => r(V::Text(as_(0).trim().to_string())),
        "IsNumeric" => r(V::Bool(as_(0).trim().parse::<f64>().is_ok())),
        "IsEmpty" | "TextIsEmpty" => r(V::Bool(as_(0).is_empty())),
        "Split" => {
            let s = as_(0);
            match s.split_once(&as_(1)) {
                Some((l, rr)) => outs(V::Bool(true), vec![(2, V::Str(l.into())), (3, V::Str(rr.into()))]),
                None => outs(V::Bool(false), vec![(2, V::Str(s)), (3, V::Str(String::new()))]),
            }
        }
        "Format" => r(V::Text(format_text(&as_(0), a1.arr()))),
        "TextFromStringTable" => r(V::Text(as_(1))),
        // UKismetStringLibrary::BuildString_*(AppendTo, Prefix, In, Suffix) = AppendTo + Prefix + In + Suffix (UE 4.26
        // KismetStringLibrary.cpp); the Prefix was dropped before, so BP_VideoSettings:UpdateResolutionDropdown@699
        // built "19201080" instead of "1920x1080" and never selected the current resolution
        "BuildString_Int" | "BuildString_Float" | "BuildString_Bool" | "BuildString_Name" | "BuildString_Object" => r(V::Str(format!("{}{}{}{}", as_(0), as_(1), as_(2), as_(3)))),
        // FText::AsTimespan (UE 4.26 Text.cpp): hours > 0 -> "H:MM:SS", else "M:SS" (UNCONFIRMED: invariant culture)
        "AsTimespan_Timespan" => {
            let s = ticks(&a0).div_euclid(TICKS_PER_SEC);
            let (h, m, sec) = (s / 3600, (s / 60) % 60, s % 60);
            r(V::Text(if h > 0 { format!("{h}:{m:02}:{sec:02}") } else { format!("{m}:{sec:02}") }))
        }
        "AsTime_DateTime" | "AsDate_DateTime" => r(V::Text(String::new())),
        // ---- FTimespan (UKismetMathLibrary; Ticks of 100 ns, Timespan.h) ----------------------------------------
        "MakeTimespan" => r(timespan(((ai(0) * 24 + ai(1)) * 60 + ai(2)) * 60 * TICKS_PER_SEC + ai(3) * TICKS_PER_SEC + ai(4) * 10_000)),
        "MakeTimespan2" => r(timespan(((ai(0) * 24 + ai(1)) * 60 + ai(2)) * 60 * TICKS_PER_SEC + ai(3) * TICKS_PER_SEC + ai(4) * 100)),
        "FromSeconds" => r(timespan((af(0) * TICKS_PER_SEC as f64).round() as i64)),
        "FromMinutes" => r(timespan((af(0) * 60.0 * TICKS_PER_SEC as f64).round() as i64)),
        "BreakTimespan" => {
            let t = ticks(&a0);
            let s = t / TICKS_PER_SEC;
            outs(V::None, vec![(1, V::Int(s / 86400)), (2, V::Int((s / 3600) % 24)), (3, V::Int((s / 60) % 60)), (4, V::Int(s % 60)), (5, V::Int((t / 10_000) % 1000))])
        }
        "GetDays" => r(V::Int(ticks(&a0) / TICKS_PER_SEC / 86400)),
        "GetHours" => r(V::Int((ticks(&a0) / TICKS_PER_SEC / 3600) % 24)),
        "GetMinutes" => r(V::Int((ticks(&a0) / TICKS_PER_SEC / 60) % 60)),
        "GetSeconds" => r(V::Int((ticks(&a0) / TICKS_PER_SEC) % 60)),
        "GetMilliseconds" => r(V::Int((ticks(&a0) / 10_000) % 1000)),
        "GetTotalSeconds" => r(V::Float(ticks(&a0) as f64 / TICKS_PER_SEC as f64)),
        "GetTotalMinutes" => r(V::Float(ticks(&a0) as f64 / (60 * TICKS_PER_SEC) as f64)),
        "GetTotalHours" => r(V::Float(ticks(&a0) as f64 / (3600 * TICKS_PER_SEC) as f64)),
        // ---- arrays (UKismetArrayLibrary; by-ref target = argument 0) -------------------------------------------
        "Array_Length" => r(V::Int(a0.arr().len() as i64)),
        "Array_LastIndex" => r(V::Int(a0.arr().len() as i64 - 1)),
        "Array_Get" => outs(V::None, vec![(2, a0.arr().get(ai(1).max(0) as usize).cloned().unwrap_or_default())]),
        "Array_IsValidIndex" => r(V::Bool(ai(1) >= 0 && (ai(1) as usize) < a0.arr().len())),
        "Array_Contains" => r(V::Bool(a0.arr().iter().any(|x| x.same(&a1)))),
        "Array_Find" => r(V::Int(a0.arr().iter().position(|x| x.same(&a1)).map_or(-1, |p| p as i64))),
        "Array_Add" => {
            let mut v = a0.arr().to_vec();
            v.push(a1);
            let n = v.len() as i64 - 1;
            outs(V::Int(n), vec![(0, V::Array(v))])
        }
        "Array_AddUnique" => {
            let mut v = a0.arr().to_vec();
            if let Some(p) = v.iter().position(|x| x.same(&a1)) {
                return outs(V::Int(p as i64), vec![]);
            }
            v.push(a1);
            let n = v.len() as i64 - 1;
            outs(V::Int(n), vec![(0, V::Array(v))])
        }
        "Array_Insert" => {
            let mut v = a0.arr().to_vec();
            let i = (ai(2).max(0) as usize).min(v.len());
            v.insert(i, a1);
            outs(V::None, vec![(0, V::Array(v))])
        }
        "Array_Set" => {
            let mut v = a0.arr().to_vec();
            let i = ai(1).max(0) as usize;
            if i < v.len() {
                v[i] = a2;
            } else if ab(3) {
                v.resize(i + 1, V::None);
                v[i] = a2;
            }
            outs(V::None, vec![(0, V::Array(v))])
        }
        "Array_Remove" => {
            let mut v = a0.arr().to_vec();
            let i = ai(1);
            if i >= 0 && (i as usize) < v.len() {
                v.remove(i as usize);
            }
            outs(V::None, vec![(0, V::Array(v))])
        }
        "Array_RemoveItem" => {
            let mut v = a0.arr().to_vec();
            let n = v.len();
            v.retain(|x| !x.same(&a1));
            outs(V::Bool(v.len() != n), vec![(0, V::Array(v))])
        }
        "Array_Clear" => outs(V::None, vec![(0, V::Array(vec![]))]),
        "Array_Resize" => {
            let mut v = a0.arr().to_vec();
            v.resize(ai(1).max(0) as usize, V::None);
            outs(V::None, vec![(0, V::Array(v))])
        }
        "Array_Append" => {
            let mut v = a0.arr().to_vec();
            v.extend_from_slice(a1.arr());
            outs(V::None, vec![(0, V::Array(v))])
        }
        "Array_Shuffle" | "Array_Swap" => r(V::None),
        "Array_Identical" => r(V::Bool(a0.same(&a1))),
        // ---- maps (UBlueprintMapLibrary) -------------------------------------------------------------------------
        "Map_Add" => {
            let mut m = match a0 {
                V::Map(m) => m,
                _ => vec![],
            };
            match m.iter_mut().find(|(k, _)| k.same(&a1)) {
                Some(e) => e.1 = a2,
                None => m.push((a1, a2)),
            }
            outs(V::None, vec![(0, V::Map(m))])
        }
        "Map_Find" => {
            let m = match &a0 {
                V::Map(m) => m.clone(),
                _ => vec![],
            };
            match m.iter().find(|(k, _)| k.same(&a1)) {
                Some((_, v)) => outs(V::Bool(true), vec![(2, v.clone())]),
                None => outs(V::Bool(false), vec![(2, V::None)]),
            }
        }
        "Map_Contains" => r(V::Bool(matches!(&a0, V::Map(m) if m.iter().any(|(k, _)| k.same(&a1))))),
        "Map_Remove" => {
            let mut m = match a0 {
                V::Map(m) => m,
                _ => vec![],
            };
            let n = m.len();
            m.retain(|(k, _)| !k.same(&a1));
            outs(V::Bool(m.len() != n), vec![(0, V::Map(m))])
        }
        "Map_Keys" => outs(V::None, vec![(1, V::Array(match &a0 {
            V::Map(m) => m.iter().map(|x| x.0.clone()).collect(),
            _ => vec![],
        }))]),
        "Map_Values" => outs(V::None, vec![(1, V::Array(match &a0 {
            V::Map(m) => m.iter().map(|x| x.1.clone()).collect(),
            _ => vec![],
        }))]),
        "Map_Length" => r(V::Int(match &a0 {
            V::Map(m) => m.len() as i64,
            _ => 0,
        })),
        "Map_Clear" => outs(V::None, vec![(0, V::Map(vec![]))]),
        // ---- system / gameplay statics ---------------------------------------------------------------------------
        "IsValid" => r(V::Bool(match &a0 {
            V::Obj(id) => vm.alive(*id),
            V::Asset(_) => true,
            _ => false,
        })),
        "IsValidClass" => r(V::Bool(matches!(a0, V::Asset(_)))),
        "PrintString" | "PrintText" | "PrintWarning" => r(V::None),
        "IsDedicatedServer" | "IsServer" => r(V::Bool(false)),
        "IsStandalone" => r(V::Bool(true)),
        // shipping PC build: UMordhauBlueprintLibrary / UKismetSystemLibrary platform queries (UNCONFIRMED: a PC
        // shipping build answers false/"Windows")
        "IsConsolePlatform" | "IsNonShippingBuildConfig" | "IsDevelopmentBuild" | "IsPackagedForDistribution_False" | "GetIsUsingController" | "IsUsingGamepad" => r(V::Bool(false)),
        "IsPackagedForDistribution" => r(V::Bool(true)),
        "GetPlatformName" => r(V::Str("Windows".into())),
        // `UMordhauUtilityLibrary::GetBuildVersion` 0x1621200: "mov eax, 0x1a" (26)
        "GetBuildVersion" => r(V::Int(0x1a)),
        // REWRITE CHOICE, not a port (user decision 2026-10-06: "disable all the errors" on Matchmaking / Server
        // Browser). The exe's `UMordhauGameInstance::CanPlayOnline` rva 0x1532960 asks the online identity / PlayFab
        // login, which the rewrite does not have, so it would be false and BP_MainMenu@6241 would raise
        // ShowNoMultiplayerErrorDialog. With no backend the rewrite answers "allowed, not banned" so both screens
        // open in their normal empty state.
        "CanPlayOnline" | "CanPlayOnlineOnly" => r(V::Bool(true)),
        "IsGlobalServerBanned" => r(V::Bool(false)),
        "GetGlobalServerBanDuration" => r(V::Int(0)),
        // Settings > Manage Mods (BP_Modlist:ExecuteUbergraph@566 / PopulatePakList@41 / @1636). `UMods::GetAllInstalledMods`
        // rva 0x14f9ff0: UMordhauUtilityLibrary::GetModsInterface; no interface -> an empty array, which is the
        // rewrite's case (no mod.io backend offline). `UMods::Process` rva 0x15131f0 ticks that interface: nothing to
        // do without one. `UMordhauUtilityLibrary::GetAllPaksPathsInCustomPaksFolder` rva 0x161f2b0 lists
        // FPaths::ProjectContentDir()/CustomPaks: the rewrite mounts no custom paks, so the list is empty (UNCONFIRMED:
        // the install's CustomPaks folder is not read)
        "GetAllInstalledMods" if class == "Mods" => r(V::Array(vec![])),
        "Process" if class == "Mods" => r(V::None),
        "GetAllPaksPathsInCustomPaksFolder" => r(V::Array(vec![])),
        // `UMordhauUtilityLibrary::GetSupportedScreenResolutions` 0x1629010: RHIGetAvailableResolutions (else the
        // primary display's size), modes > 1023 x 719 only, each "%dx%d %d:%d" with the aspect named 4:3 / 5:3 / 5:4 /
        // 16:9 (+-0.01), 16:10 (+-0.04), 21:9 (+-0.06) else gcd-reduced, added unique. The adapter's mode list is
        // not queried by the rewrite: the common desktop modes stand in (UNCONFIRMED list)
        "GetSupportedScreenResolutions" => {
            const MODES: [(i64, i64); 20] = [
                (1024, 768), (1152, 864), (1280, 720), (1280, 768), (1280, 800), (1280, 960), (1280, 1024), (1360, 768), (1366, 768), (1440, 900),
                (1600, 900), (1600, 1200), (1680, 1050), (1920, 1080), (1920, 1200), (2560, 1080), (2560, 1440), (2560, 1600), (3440, 1440), (3840, 2160),
            ];
            let gcd = |mut a: i64, mut b: i64| {
                while b != 0 {
                    let t = a % b;
                    a = b;
                    b = t;
                }
                a.max(1)
            };
            let mut out: Vec<V> = vec![];
            for (w, h) in MODES {
                if w <= 0x3ff || h <= 0x2cf {
                    continue;
                }
                let r = w as f64 / h as f64;
                let (ax, ay) = if (r - 4.0 / 3.0).abs() <= 0.01 {
                    (4, 3)
                } else if (r - 5.0 / 3.0).abs() <= 0.01 {
                    (5, 3)
                } else if (r - 1.25).abs() <= 0.01 {
                    (5, 4)
                } else if (r - 16.0 / 9.0).abs() <= 0.01 {
                    (16, 9)
                } else if (r - 1.6).abs() <= 0.04 {
                    (16, 10)
                } else if (r - 7.0 / 3.0).abs() <= 0.06 {
                    (21, 9)
                } else {
                    let g = gcd(w, h);
                    (w / g, h / g)
                };
                let s = format!("{w}x{h} {ax}:{ay}");
                if !out.iter().any(|x| x.s() == s) {
                    out.push(V::Str(s));
                }
            }
            let ok = !out.is_empty();
            outs(V::Bool(ok), vec![(0, V::Array(out))])
        }
        // `UMordhauUtilityLibrary::SortPlayers` 0x163f310: a copy sorted by MordhauPlayerStateSortPredicate 0x162e7d0:
        // Score desc, Kills desc, Deaths asc, GetPlayerName asc (wchar compare), PlayerId asc
        "SortPlayers" => {
            let mut v: Vec<V> = a0.arr().to_vec();
            let key = |x: &V| -> (f64, i64, i64, String, i64) {
                match x.obj() {
                    Some(o) if vm.alive(o) => (vm.prop(o, "Score").f(), vm.prop(o, "Kills").i(), vm.prop(o, "Deaths").i(), vm.prop(o, "PlayerName").s(), vm.prop(o, "PlayerId").i()),
                    _ => (0.0, 0, 0, String::new(), 0),
                }
            };
            v.sort_by(|x, y| {
                let (a, b) = (key(x), key(y));
                b.0.partial_cmp(&a.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(b.1.cmp(&a.1))
                    .then(a.2.cmp(&b.2))
                    .then_with(|| a.3.encode_utf16().cmp(b.3.encode_utf16()))
                    .then(a.4.cmp(&b.4))
            });
            r(V::Array(v))
        }
        // `UMordhauUtilityLibrary::GetPing` 0x1624d00: the local PlayerState's ExactPing * 0.001 (seconds; bUseMedian
        // reads AMordhauPlayerState's median field, the same value here)
        "GetPing" => {
            let ps = vm.world.get("ps").copied();
            r(V::Float(ps.map(|p| vm.prop(p, "Ping").i() as f64 * 4.0 * 0.001).unwrap_or(0.0)))
        }
        // `UMordhauUtilityLibrary::GetRankFromXP` 0x16286f0: XP <= 0 -> 1; above, the GetXPFromRank 0x162a7c0 curve
        // (not ported: offline profiles have no XP -> 1, UNCONFIRMED for XP > 0)
        "GetRankFromXP" => r(V::Int(1)),
        // `UMordhauUtilityLibrary::GetIsPeasant` 0x16234b0: (Profile.SkillsCustomization.Perks & 8) != 0
        "GetIsPeasant" => r(V::Bool(a0.field("SkillsCustomization").field("Perks").i() & 8 != 0)),
        // `IsDevelopmentEnvironment` 0x162bd90 (UOnlineUtilities dev-env check): a shipped install -> false (UNCONFIRMED)
        "IsDevelopmentEnvironment" => r(V::Bool(false)),
        // `IsReflexAvailable` 0x162c3a0 asks the NVIDIA Reflex plugin: none in the rewrite -> false
        "IsReflexAvailable" => r(V::Bool(false)),
        // FPlayFabPlayer::IsValid (0x127fed0 forwards): a non-empty PlayFab id; offline players carry none
        "IsValid_PlayFabPlayer" => r(V::Bool(!a0.field("PlayFabId").s().is_empty())),
        "Equal_PlayFabPlayer" => r(V::Bool(!a0.field("PlayFabId").s().is_empty() && a0.field("PlayFabId").s() == a.get(1).cloned().unwrap_or_default().field("PlayFabId").s())),
        // `UMordhauUtilityLibrary::GetVersionName` 0x16298b0: UOnlineUtilities::GetReleaseVersion() < 1 -> "Development",
        // == 6 -> "Beta", else "Release" (the shipped build's ReleaseVersion taken as a release: UNCONFIRMED)
        "GetVersionName" => r(V::Str("Release".into())),
        // `UMordhauUtilityLibrary::GetPlatformSpecific` 0x7bf3e0 (also IsClient / IsStandalone): ICF alias of
        // IAccessibleProperty::IsReadOnly, "mov al, 1; ret"
        "GetPlatformSpecific" | "IsClient" => r(V::Int(1)),
        "GetPlatform" => r(V::Int(0)),
        "GetGameTimeInSeconds" | "GetRealTimeSeconds" | "GetTimeSeconds" | "GetAudioTimeSeconds" | "GetAccurateRealTime" | "GetServerWorldTimeSeconds" => r(V::Float(vm.time)),
        "GetWorldDeltaSeconds" => r(V::Float(1.0 / 60.0)),
        "GetDisplayName" | "GetObjectName" => r(V::Str(match &a0 {
            V::Obj(id) => vm.o(*id).name.clone(),
            V::Asset(x) => x.name.clone(),
            _ => String::new(),
        })),
        "GetClass" => r(match &a0 {
            V::Obj(id) => {
                let c = &vm.o(*id).class;
                V::Asset(std::sync::Arc::new(ObjRef { package: c.package.clone(), name: c.name.clone(), class: "Class".into(), outer: String::new() }))
            }
            _ => V::None,
        }),
        "Delay" | "RetriggerableDelay" => {
            // FLatentActionManager: a Delay with the same UUID still pending is ignored; Retriggerable resets it
            let li = &a2;
            let uuid = li.field("UUID").i();
            let target = li.field("CallbackTarget").obj().unwrap_or(t);
            let func = li.field("ExecutionFunction").s();
            let at = vm.time + af(1);
            if let Some(l) = vm.latent.iter_mut().find(|l| l.uuid == uuid && l.obj == target) {
                if name == "RetriggerableDelay" {
                    l.at = at;
                }
                return r(V::None);
            }
            vm.latent.push(Latent { at, obj: target, func, linkage: li.field("Linkage").i(), uuid, period: 0.0 });
            r(V::None)
        }
        "K2_SetTimerDelegate" | "K2_SetTimer" => {
            let (o, f) = match (&a0, name) {
                (V::Delegate(o, f), _) => (*o, f.clone()),
                (_, "K2_SetTimer") => (if let V::Obj(o) = a0 { o } else { t }, as_(1)),
                _ => return r(V::None),
            };
            let (time, looping) = if name == "K2_SetTimer" { (af(2), ab(3)) } else { (af(1), ab(2)) };
            let uuid = -((vm.latent.len() as i64) + 1000 * o as i64) - 1;
            vm.latent.retain(|l| !(l.obj == o && l.func == f && l.linkage == -1));
            if time > 0.0 {
                vm.latent.push(Latent { at: vm.time + time, obj: o, func: f.clone(), linkage: -1, uuid, period: if looping { time } else { 0.0 } });
            }
            r(V::st(&[("Handle", V::Int(uuid)), ("Object", V::Obj(o)), ("Function", V::Name(f))]))
        }
        "K2_ClearTimerHandle" | "K2_ClearAndInvalidateTimerHandle" | "K2_PauseTimerHandle" => {
            let h = a1.field("Handle").i();
            vm.latent.retain(|l| l.uuid != h);
            outs(V::None, vec![(1, V::st(&[("Handle", V::Int(0))]))])
        }
        "K2_ClearTimer" | "K2_ClearTimerDelegate" => {
            if let V::Delegate(o, f) = &a0 {
                vm.latent.retain(|l| !(l.obj == *o && &l.func == f));
            }
            r(V::None)
        }
        "K2_IsTimerActiveHandle" | "K2_TimerExistsHandle" => {
            let h = a1.field("Handle").i();
            r(V::Bool(vm.latent.iter().any(|l| l.uuid == h)))
        }
        // (first-person r3) the escape menu's pawn calls (BP_ArmoryTypeSelection ubergraph @2103 RequestSuicide, @2470
        // CycleCamera): carried out by the game (mh-runtime ui_pawn)
        "RequestSuicide" | "CycleCamera" => {
            vm.actions.push(Action::Pawn(name.to_string()));
            r(V::None)
        }
        // (first-person r3) UDestroyMordhauServerSession::DestroyMordhauServerSession rva=0x15cf5a0: the async proxy;
        // Activate rva=0x15c0b30 broadcasts OnFailure at once when there is no session interface (an offline game), and
        // BP_MainMenu:QuitMatch's OnFailure runs ExecuteConsoleCommand("disconnect") (ubergraph @3191)
        "DestroyMordhauServerSession" => {
            let c = vm.native_class("DestroyMordhauServerSession");
            let id = vm.new_obj(c, "DestroyMordhauServerSession");
            r(V::Obj(id))
        }
        "Activate" if vm.alive(t) && vm.o(t).class.native() == "DestroyMordhauServerSession" => {
            if let V::Multi(m) = vm.prop(t, "OnFailure") {
                for (o, f) in m {
                    vm.call_named(o, &f, vec![]);
                }
            }
            r(V::None)
        }
        "ExecuteConsoleCommand" => {
            vm.actions.push(Action::Console(as_(1)));
            r(V::None)
        }
        "QuitGame" => {
            vm.actions.push(Action::Quit);
            r(V::None)
        }
        "OpenLevel" => {
            // UGameplayStatics::OpenLevel(WorldContextObject, LevelName, bAbsolute, Options)
            vm.actions.push(Action::OpenLevel { map: as_(1), options: as_(3) });
            r(V::None)
        }
        "PlaySound2D" | "SpawnSound2D" | "PlaySound" => {
            if let V::Asset(s) = &a1 {
                vm.actions.push(Action::Sound(format!("{}", s.package)));
            }
            r(V::None)
        }
        "GetPlayerController" | "GetOwningPlayer" | "GetFirstLocalPlayerController" | "GetMordhauPlayerController" => r(world(vm, "pc")),
        // the possessed pawn (AController::Pawn; None before the first spawn and while dead / spectating: host.rs
        // possess / unpossess)
        "GetPlayerPawn" | "GetViewTargetCharacter" | "GetPlayerCharacter" | "GetOwningPlayerPawn" | "K2_GetPawn" | "GetControlledPawn" => r(vm.world.get("pc").map(|&pc| vm.prop(pc, "Pawn")).unwrap_or_default()),
        "GetGameState" => r(world(vm, "gs")),
        "GetGameInstance" | "GetMordhauGameInstance" => r(world(vm, "gi")),
        "GetHUD" | "GetMordhauHUD" => r(world(vm, "hud")),
        "GetMordhauGameUserSettings" | "GetGameUserSettings" => r(world(vm, "settings")),
        "GetMordhauSingleton" => r(world(vm, "singleton")),
        "GetMordhauInput" => r(world(vm, "input")),
        "GetPlayerState" | "GetMordhauPlayerState" => r(world(vm, "ps")),
        // PlayFab backend stand-ins (the rewrite is offline): inventory / stats / unlock recipes report available
        // (UNCONFIRMED: the backend is replaced, host.rs fills the flags)
        "GetMordhauInventory" => r(world(vm, "inventory")),
        "GetMordhauStats" => r(world(vm, "stats")),
        "GetPlayFabID" => r(V::Str("offline".into())),
        // UMordhauUtilityLibrary::GetMapInfo 0x1622650 -> UMordhauGameInstance::GetMapInfo 0x1540690 (menu_data.rs)
        "GetMapInfo" => {
            let gi = vm.world.get("gi").copied().unwrap_or(0);
            let p = a.get(1).map(V::s).unwrap_or_default();
            r(crate::menu_data::map_info(vm, gi, &p))
        }
        // UMordhauGameInstance::ClientTravel 0x1535ce0 (menu_data.rs travel_options)
        "ClientTravel" => {
            let (map, options) = crate::menu_data::travel_options(&as_(0), ai(1));
            vm.actions.push(Action::OpenLevel { map, options });
            r(V::Bool(true))
        }
        // synchronous soft-reference loads (LoadAsset_Blocking / Mordhau's LoadAsset): the referenced package object,
        // class from the package's first export (mh-pak first_export_class)
        "LoadAsset" | "LoadAsset_Blocking" | "LoadClassAsset_Blocking" | "Conv_SoftObjectReferenceToObject" | "Conv_SoftClassReferenceToClass" => {
            let p = match &a0 {
                V::Struct(_) => a0.field("AssetPathName").s(),
                V::Asset(_) => return r(a0.clone()),
                v => v.s(),
            };
            if p.is_empty() || p == "None" {
                return r(V::None);
            }
            let (pk, n) = p.rsplit_once('.').unwrap_or((&p, ""));
            let pk = crate::kismet::content_path(pk);
            let cls = vm.src.as_ref().map(|s| mh_pak::first_export_class(&s.vfs, &pk)).unwrap_or_default();
            r(V::Asset(std::sync::Arc::new(crate::kismet::ObjRef { package: pk, name: n.to_string(), class: cls, outer: String::new() })))
        }
        "Conv_SoftObjectPathToString" | "Conv_SoftObjectReferenceToString" | "Conv_SoftClassReferenceToString" => r(V::Str(match &a0 {
            V::Struct(_) => a0.field("AssetPathName").s(),
            V::Asset(x) => format!("{}.{}", x.package, x.name),
            v => v.s(),
        })),
        "GetPlayerCameraManager" => r(world(vm, "camera")),
        "GetIsInMapView" => r(V::Bool(false)),
        "GetGameMode" | "GetDriver" | "GetPlayFabPlayer" => r(V::None),
        "IsLocalPlayerController" | "IsLocalController" => r(V::Bool(true)),
        "GetMapName" | "GetCurrentLevelName" => r(V::Str(vm.world.get("map").map(|&m| vm.prop(m, "Name").s()).unwrap_or_default())),
        "GetAllActorsOfClass" => outs(V::None, vec![(2, V::Array(vec![]))]),
        "IsGamePaused" => r(V::Bool(false)),
        // UWidgetBlueprintLibrary::SetInputMode_UIOnlyEx / _GameAndUIEx (UMordhauInput::CustomSetInputModeUIOnly
        // rva=0x15893b0 / CustomSetInputModeGameAndUI rva=0x15892f0 call them): FInputModeUIOnly / FInputModeGameAndUI
        // ::ApplyInputMode gives the user focus to InWidgetToFocus when one is passed (BP_MainMenu:Show@293 passes self,
        // so its OnPreviewKeyDown gets Escape: HandleInput -> AskHUDToHideUs)
        "SetInputMode_UIOnlyEx" | "SetInputMode_GameAndUIEx" | "SetInputMode_UIOnly" | "SetInputMode_GameAndUI" | "CustomSetInputModeUIOnly" | "CustomSetInputModeGameAndUI" => {
            if let Some(w) = a1.obj() {
                if vm.visible_widget_path(w).is_some() {
                    vm.focus = Some(w);
                }
            }
            r(V::None)
        }
        // FInputModeGameOnly::ApplyInputMode / SetFocusToGameViewport: the user focus goes to the game viewport (no widget);
        // CustomSetInputModeGameOnly rva=0x1589370 -> SetInputMode_GameOnly
        "SetInputMode_GameOnly" | "CustomSetInputModeGameOnly" | "SetFocusToGameViewport" => {
            vm.focus = None;
            r(V::None)
        }
        "SetGamePaused" | "SetIgnoreLookInput" | "SetIgnoreMoveInput" | "ResetIgnoreLookInput" | "ResetIgnoreMoveInput" | "ResetIgnoreInputFlags" | "FlushPressedKeys" | "StopAllCameraShakes" | "DisableVirtualCursor" | "EnableVirtualCursor" | "SetMouseLocation" => r(V::None),
        "Key_IsValid" => r(V::Bool(key_name(&a0) != "None")),
        "Key_GetDisplayName" => r(V::Text(crate::input::key_display_name(&a0.field("KeyName").s()))),
        "GetKey" => r(a0.field("Key").clone()),
        "PointerEvent_GetEffectingButton" => r(a0.field("EffectingButton").clone()),
        "PointerEvent_GetScreenSpacePosition" => r(a0.field("ScreenSpacePosition").clone()),
        "GetInputEventFromKeyEvent" | "GetInputEventFromPointerEvent" | "GetInputEventFromCharacterEvent" | "GetInputEventFromNavigationEvent" => r(a0),
        "InputEvent_IsRepeat" => r(V::Bool(a0.field("bIsRepeat").truthy())),
        "InputEvent_IsShiftDown" | "InputEvent_IsControlDown" | "InputEvent_IsAltDown" | "InputEvent_IsCommandDown" | "InputEvent_IsLeftShiftDown" | "InputEvent_IsLeftControlDown" | "InputEvent_IsLeftAltDown" => r(V::Bool(false)),
        "Key_IsMouseButton" => r(V::Bool(a0.field("KeyName").s().ends_with("MouseButton"))),
        "Key_IsGamepadKey" => r(V::Bool(a0.field("KeyName").s().starts_with("Gamepad"))),
        "Key_IsKeyboardKey" => {
            let k = a0.field("KeyName").s();
            r(V::Bool(!k.ends_with("MouseButton") && !k.starts_with("Gamepad") && !k.starts_with("Mouse") && k != "None"))
        }
        // static UMordhauInput accessors on a mapping struct: `GetActionKey` 0x1594a50 / `GetActionName` 0x1594a90
        // (FInputActionKeyMapping Key / ActionName), `GetAxisScale` 0x1432180 (FInputAxisKeyMapping Scale); GetAxisKey /
        // GetAxisName the axis mapping's Key / AxisName (UNCONFIRMED: assumed the same one-field loads)
        // `AMordhauEquipment::GetAmmo` 0x153ecc0 (the Ammo byte); `GetCurrentMaxAmmo` 0x153f1a0 adjusts MaxAmmo by the
        // owner's perks (not applied: UNCONFIRMED)
        "GetAmmo" if vm.alive(t) && vm.o(t).class.isa("MordhauEquipment") => r(V::Int(vm.prop(t, "Ammo").i())),
        "GetCurrentMaxAmmo" | "GetMaxAmmo" if vm.alive(t) && vm.o(t).class.isa("MordhauEquipment") => r(V::Int(vm.prop(t, "MaxAmmo").i())),
        "GetActionKey" | "GetAxisKey" => r(a0.field("Key").clone()),
        "GetActionName" => r(a0.field("ActionName").clone()),
        "GetAxisName" => r(a0.field("AxisName").clone()),
        "GetAxisScale" => r(V::Float(a0.field("Scale").f())),
        // `UMordhauInput::GetAxisOppositeDirectionName` 0x1594e10: a name ending Forward / Up / Right gets the three
        // replacements towards Backward / Down / Left, and the reverse (the replaced strings are .rdata literals,
        // read as these words: UNCONFIRMED)
        "GetAxisOppositeDirectionName" => {
            let n = a0.s();
            let sw = |n: &str, from: [&str; 3], to: [&str; 3]| from.iter().zip(to).fold(n.to_string(), |acc, (f, t)| acc.replace(f, t));
            let fwd = ["Forward", "Up", "Right"];
            let back = ["Backward", "Down", "Left"];
            r(V::Name(if fwd.iter().any(|x| n.ends_with(x)) {
                sw(&n, fwd, back)
            } else if back.iter().any(|x| n.ends_with(x)) {
                sw(&n, back, fwd)
            } else {
                n
            }))
        }
        // UKismetInputLibrary::Key_IsAxis1D: the 1D axis keys (UE 4.26 EKeys: mouse X / Y / wheel, gamepad sticks and
        // trigger axes)
        "Key_IsAxis1D" => {
            let k = a0.field("KeyName").s();
            r(V::Bool(matches!(k.as_str(), "MouseX" | "MouseY" | "MouseWheelAxis" | "Gamepad_LeftX" | "Gamepad_LeftY" | "Gamepad_RightX" | "Gamepad_RightY" | "Gamepad_LeftTriggerAxis" | "Gamepad_RightTriggerAxis")))
        }
        // `UMordhauUtilityLibrary::AreConfirmCancelSwapped` 0x7bf520: "xor al, al; ret"
        "AreConfirmCancelSwapped" => r(V::Bool(false)),
        // `UMordhauUtilityLibrary::ToggleNavigation` 0x1640e40: installs a Slate FNavigationConfig (gamepad / key
        // navigation) for a local controller; no visible effect here
        "ToggleNavigation" => r(V::None),
        // UKismetNodeHelperLibrary (K2Node_SwitchEnum / ForEachEnum / enum literals): values pass through, the
        // friendly name is the enum's DisplayNameMap entry (UserDefinedEnum) else the bare entry name
        "GetValidValue" => r(V::Int(ai(1))),
        "GetEnumeratorValueFromIndex" => r(V::Int(ai(1))),
        "GetEnumeratorName" | "GetEnumeratorUserFriendlyName" => r(V::Str(enum_name(vm, &a0, ai(1), name == "GetEnumeratorUserFriendlyName"))),
        "Handled" => r(V::st(&[("Handled", V::Bool(true))])),
        "Unhandled" => r(V::st(&[("Handled", V::Bool(false))])),
        "CaptureMouse" | "ReleaseMouseCapture" | "LockMouse" | "UnlockMouse" | "SetUserFocus" | "SetMousePosition" | "ClearUserFocus" => r(a0),
        // ---- widget creation / viewport -------------------------------------------------------------------------
        "Create" | "CreateWidget" => {
            // UWidgetBlueprintLibrary::Create(WorldContextObject, WidgetType, OwningPlayer)
            let Some(c) = (match &a1 {
                V::Asset(x) => Some(x.clone()),
                _ => None,
            }) else {
                return r(V::None);
            };
            let cls = vm.class(&c);
            let n = c.name.trim_end_matches("_C").to_string();
            let id = vm.create(cls, &n);
            r(V::Obj(id))
        }
        "SpawnObject" => {
            let Some(c) = (match &a0 {
                V::Asset(x) => Some(x.clone()),
                _ => None,
            }) else {
                return r(V::None);
            };
            let cls = vm.class(&c);
            let id = vm.new_obj(cls, &c.name);
            r(V::Obj(id))
        }
        "GetAllWidgetsOfClass" => {
            // (WorldContextObject, out FoundWidgets, WidgetClass, TopLevelOnly)
            let cn = match &a2 {
                V::Asset(x) => x.name.clone(),
                _ => String::new(),
            };
            let mut found = vec![];
            let roots: Vec<_> = vm.viewport.iter().map(|x| x.0).collect();
            for root in roots {
                let mut d = vec![];
                if a.get(3).map(V::truthy).unwrap_or(true) {
                    d.push(root);
                } else {
                    vm.descendants(root, &mut d);
                }
                for w in d {
                    if vm.o(w).class.isa(&cn) {
                        found.push(V::Obj(w));
                    }
                }
            }
            outs(V::None, vec![(1, V::Array(found))])
        }
        "GetViewportSize" => r(world(vm, "viewport").obj().map(|v| vm.prop(v, "Size")).unwrap_or_default()),
        "GetViewportScale" => r(V::Float(world(vm, "viewport").obj().map(|v| vm.prop(v, "Scale").f()).unwrap_or(1.0))),
        "GetMousePositionOnViewport" | "GetMousePositionOnPlatform" => r(world(vm, "viewport").obj().map(|v| vm.prop(v, "Mouse")).unwrap_or_default()),
        "RemoveAllWidgets" => {
            vm.viewport.clear();
            r(V::None)
        }
        "SlotAsOverlaySlot" => r(slot_as(vm, &a0, "OverlaySlot")),
        "SlotAsCanvasSlot" => r(slot_as(vm, &a0, "CanvasPanelSlot")),
        "SlotAsHorizontalBoxSlot" => r(slot_as(vm, &a0, "HorizontalBoxSlot")),
        "SlotAsVerticalBoxSlot" => r(slot_as(vm, &a0, "VerticalBoxSlot")),
        "SlotAsBorderSlot" => r(slot_as(vm, &a0, "BorderSlot")),
        "SlotAsSizeBoxSlot" => r(slot_as(vm, &a0, "SizeBoxSlot")),
        "SlotAsScrollBoxSlot" => r(slot_as(vm, &a0, "ScrollBoxSlot")),
        "SlotAsUniformGridSlot" => r(slot_as(vm, &a0, "UniformGridSlot")),
        "SlotAsGridSlot" => r(slot_as(vm, &a0, "GridSlot")),
        "SlotAsWrapBoxSlot" => r(slot_as(vm, &a0, "WrapBoxSlot")),
        "SlotAsSafeBoxSlot" => r(slot_as(vm, &a0, "SafeZoneSlot")),
        "SlotAsScaleBoxSlot" => r(slot_as(vm, &a0, "ScaleBoxSlot")),
        "MakeBrushFromTexture" => r(V::st(&[
            ("ResourceObject", a0.clone()),
            ("ImageSize", V::st(&[("X", V::Float(if ai(1) > 0 { af(1) } else { 32.0 })), ("Y", V::Float(if ai(2) > 0 { af(2) } else { 32.0 }))])),
            ("DrawAs", V::Int(3)),
        ])),
        // UKismetMaterialLibrary::CreateDynamicMaterialInstance(WorldContext, Parent, OptionalName, ...): a
        // MaterialInstanceDynamic object holding its parent and the parameters set on it
        "CreateDynamicMaterialInstance" => {
            let c = vm.native_class("MaterialInstanceDynamic");
            let id = vm.new_obj(c, "MID");
            vm.set(id, "Parent", a1.clone());
            r(V::Obj(id))
        }
        "MakeBrushFromMaterial" | "MakeBrushFromAsset" if a.len() >= 3 => r(V::st(&[
            ("ResourceObject", a0.clone()),
            ("ImageSize", V::st(&[("X", V::Float(if ai(1) > 0 { af(1) } else { 32.0 })), ("Y", V::Float(if ai(2) > 0 { af(2) } else { 32.0 }))])),
            ("DrawAs", V::Int(3)),
        ])),
        "MakeBrushFromMaterial" | "MakeBrushFromAsset" => r(V::st(&[("ResourceObject", a0.clone()), ("ImageSize", V::st(&[("X", V::Float(32.0)), ("Y", V::Float(32.0))])), ("DrawAs", V::Int(3))])),
        "NoResourceBrush" => r(V::st(&[("DrawAs", V::Int(0))])),
        "SetBrushResourceToTexture" => {
            let mut b = a0.clone();
            if let V::Struct(m) = &mut b {
                m.insert("ResourceObject".into(), a1);
            }
            outs(V::None, vec![(0, b)])
        }
        _ => widget_method(vm, t, class, name, a),
    }
}

/// UMG widget / slot methods on target `t`
fn widget_method(vm: &mut Vm, t: crate::model::Id, _class: &str, name: &str, a: &[V]) -> Option<NRet> {
    let a0 = a.first().cloned().unwrap_or_default();
    let a1 = a.get(1).cloned().unwrap_or_default();
    if !vm.alive(t) {
        return proxy(vm, t, name, a);
    }
    let set = |vm: &mut Vm, k: &str, v: V| {
        vm.set(t, k, v);
        r(V::None)
    };
    let setf = |vm: &mut Vm, path: &[&str], v: V| {
        let mut cur = vm.prop(t, path[0]);
        fn put(s: &mut V, p: &[&str], v: V) {
            if !matches!(s, V::Struct(_)) {
                *s = V::Struct(Box::default());
            }
            if let V::Struct(m) = s {
                if p.len() == 1 {
                    m.insert(p[0].to_string(), v);
                } else {
                    let e = m.entry(p[0].to_string()).or_default();
                    put(e, &p[1..], v);
                }
            }
        }
        if path.len() == 1 {
            cur = v;
        } else {
            put(&mut cur, &path[1..], v);
        }
        vm.set(t, path[0], cur);
        r(V::None)
    };
    let native = vm.o(t).class.native().to_string();
    // UE 4.26 UMG: a native setter replaces the Slate attribute the binding fed (UImage::SetBrush ->
    // MyImage->SetImage(&Brush), UTextBlock::SetText -> TextDelegate.Unbind(), UWidget::SetVisibility ->
    // SafeWidget->SetVisibility, UImage/UTextBlock SetColorAndOpacity, UProgressBar::SetPercent): from then on the
    // set value stands and the binding no longer runs (slate.rs run_bindings skips "__unbound:<Property>")
    let unbinds: &[&str] = match name {
        "SetBrush" | "SetBrushFromTexture" | "SetBrushFromTextureDynamic" | "SetBrushFromMaterial" | "SetBrushFromAsset" | "SetBrushResourceObject" | "SetBrushFromSoftTexture" | "SetBrushFromSoftMaterial" | "SetBrushTintColor" | "SetBrushSize" => &["Brush"],
        "SetText" => &["Text"],
        "SetVisibility" => &["Visibility"],
        "SetColorAndOpacity" | "SetOpacity" => &["ColorAndOpacity"],
        "SetPercent" => &["Percent"],
        "SetFillColorAndOpacity" => &["FillColorAndOpacity"],
        "SetIsEnabled" => &["bIsEnabled"],
        "SetToolTipText" => &["ToolTipText"],
        "SetBrushColor" => &["BrushColor"],
        "SetContentColorAndOpacity" => &["ContentColorAndOpacity"],
        _ => &[],
    };
    for p in unbinds {
        vm.set(t, &format!("__unbound:{p}"), V::Bool(true));
    }
    match name {
        "SetVisibility" => {
            // UWidget::SetVisibility: a change broadcasts OnVisibilityChanged(InVisibility) (UE 4.26 Widget.cpp;
            // BP_TitleScreen binds it in Construct: ExecuteUbergraph_BP_TitleScreen@826..@849)
            let old = vm.prop(t, "Visibility").i();
            let new = a0.i();
            vm.set(t, "Visibility", V::Int(new));
            if old != new {
                if let V::Multi(m) = vm.prop(t, "OnVisibilityChanged") {
                    for (o, f) in m {
                        vm.call_named(o, &f, vec![V::Int(new)]);
                    }
                }
            }
            r(V::None)
        }
        "GetVisibility" => r(V::Int(vm.prop(t, "Visibility").i())),
        "IsVisible" => r(V::Bool(vis_visible(vm.prop(t, "Visibility").i()) && is_shown(vm, t))),
        "SetRenderOpacity" => set(vm, "RenderOpacity", V::Float(a0.f())),
        "GetRenderOpacity" => r(V::Float(match vm.prop(t, "RenderOpacity") {
            V::None => 1.0,
            v => v.f(),
        })),
        "SetIsEnabled" => set(vm, "bIsEnabled", V::Bool(a0.truthy())),
        "GetIsEnabled" => r(V::Bool(!matches!(vm.prop(t, "bIsEnabled"), V::Bool(false)))),
        "SetToolTipText" => set(vm, "ToolTipText", a0),
        "SetRenderTranslation" => setf(vm, &["RenderTransform", "Translation"], a0),
        "SetRenderScale" => setf(vm, &["RenderTransform", "Scale"], a0),
        "SetRenderShear" => setf(vm, &["RenderTransform", "Shear"], a0),
        "SetRenderTransformAngle" => setf(vm, &["RenderTransform", "Angle"], a0),
        "SetRenderTransform" => set(vm, "RenderTransform", a0),
        "SetRenderTransformPivot" => set(vm, "RenderTransformPivot", a0),
        "SetClipping" => set(vm, "Clipping", a0),
        "SetCursor" | "ForceLayoutPrepass" | "InvalidateLayoutAndVolatility" | "SetNavigationRule" | "SetNavigationRuleBase" | "SetNavigationRuleExplicit" | "SetNavigationRuleCustom" | "ForceVolatile" | "SetAllNavigationRules" => r(V::None),
        "SetKeyboardFocus" | "SetFocus" | "SetUserFocus" => {
            if vm.visible_widget_path(t).is_some() {
                vm.focus = Some(t);
            }
            r(V::None)
        }
        "HasKeyboardFocus" | "HasUserFocus" | "HasAnyUserFocus" | "HasFocusedDescendants" | "HasUserFocusedDescendants" => {
            let f = vm.focus;
            r(V::Bool(match f {
                Some(f) if f == t => true,
                Some(f) if name.contains("Descendants") => {
                    let mut d = vec![];
                    vm.descendants(t, &mut d);
                    d.contains(&f)
                }
                _ => false,
            }))
        }
        "IsHovered" => r(V::Bool(vm.prop(t, "__hovered").truthy())),
        "IsPressed" => r(V::Bool(vm.prop(t, "__pressed").truthy())),
        "RemoveFromParent" | "RemoveFromViewport" => {
            remove_from_parent(vm, t);
            r(V::None)
        }
        "GetParent" => r(vm.o(t).parent_slot.and_then(|s| vm.o(s).panel).map(V::Obj).unwrap_or_default()),
        "GetDesiredSize" => r(vm.prop(t, "__desired")),
        "GetCachedGeometry" | "GetTickSpaceGeometry" | "GetPaintSpaceGeometry" => r(V::Struct(Box::default())),
        // UUserWidget
        "AddToViewport" | "AddToPlayerScreen" => {
            let z = a0.i();
            vm.viewport.retain(|x| x.0 != t);
            vm.viewport.push((t, z));
            vm.construct(t);
            r(V::Bool(true))
        }
        "IsInViewport" => r(V::Bool(vm.viewport.iter().any(|x| x.0 == t))),
        "SetPositionInViewport" => set(vm, "__vp_pos", a0),
        "SetDesiredSizeInViewport" => set(vm, "__vp_size", a0),
        "SetAlignmentInViewport" => set(vm, "__vp_align", a0),
        "SetAnchorsInViewport" => set(vm, "__vp_anchors", a0),
        "SetOwningPlayer" | "SetOwningLocalPlayer" | "SetInputActionPriority" | "SetInputActionBlocking" | "ListenForInputAction" | "StopListeningForAllInputActions" | "StopListeningForInputAction" | "RegisterInputComponent" | "UnregisterInputComponent" => r(V::None),
        "PlayAnimation" => {
            // (InAnimation, StartAtTime, NumLoopsToPlay, PlayMode, PlaybackSpeed)
            let rev = a.get(3).map(V::i).unwrap_or(0) == 1;
            play(vm, t, &a0, a.get(1).map(V::f).unwrap_or(0.0), a.get(2).map(V::i).unwrap_or(1), rev, a.get(4).map(V::f).unwrap_or(1.0));
            r(V::None)
        }
        "PlayAnimationForward" | "PlayAnimationReverse" => {
            play(vm, t, &a0, 0.0, 1, name.ends_with("Reverse"), a.get(1).map(V::f).unwrap_or(1.0));
            r(V::None)
        }
        "PlayAnimationTimeRange" => {
            play(vm, t, &a0, a.get(1).map(V::f).unwrap_or(0.0), a.get(3).map(V::i).unwrap_or(1), a.get(4).map(V::i).unwrap_or(0) == 1, a.get(5).map(V::f).unwrap_or(1.0));
            r(V::None)
        }
        "StopAnimation" | "PauseAnimation" => {
            let n = anim_name(&a0);
            vm.playing.retain(|p| !(p.widget == t && p.anim == n));
            r(V::Float(0.0))
        }
        "StopAllAnimations" => {
            vm.playing.retain(|p| p.widget != t);
            r(V::None)
        }
        "IsAnimationPlaying" => {
            let n = anim_name(&a0);
            r(V::Bool(vm.playing.iter().any(|p| p.widget == t && p.anim == n)))
        }
        "IsAnyAnimationPlaying" => r(V::Bool(vm.playing.iter().any(|p| p.widget == t))),
        "GetAnimationCurrentTime" => {
            let n = anim_name(&a0);
            r(V::Float(vm.playing.iter().find(|p| p.widget == t && p.anim == n).map_or(0.0, |p| vm.time - p.start)))
        }
        "GetEndTime" => {
            // UWidgetAnimation::GetEndTime (called on the animation object): its length
            r(V::Float(0.0))
        }
        "BindToAnimationFinished" | "BindToAnimationEvent" => {
            let n = anim_name(&a0);
            let key = format!("__anim_finished:{n}");
            let mut m = match vm.prop(t, &key) {
                V::Multi(m) => m,
                _ => vec![],
            };
            if let V::Delegate(o, f) = &a1 {
                m.retain(|(x, g)| !(x == o && g == f));
                m.push((*o, f.clone()));
            }
            set(vm, &key, V::Multi(m))
        }
        "UnbindAllFromAnimationFinished" | "UnbindFromAnimationFinished" => {
            let key = format!("__anim_finished:{}", anim_name(&a0));
            set(vm, &key, V::Multi(vec![]))
        }
        "SetForegroundColor" => set(vm, "ForegroundColor", a0),
        // UMordhauTitleScreen::Setup 0x163e4b0: KeyToContinue / GamepadKeyToContinue = the arguments (the online
        // delegate bindings it also makes are not modelled)
        "Setup" if vm.o(t).class.isa("MordhauTitleScreen") => {
            vm.set(t, "KeyToContinue", a0);
            set(vm, "GamepadKeyToContinue", a1)
        }
        "OnShown" | "OnUserConfirmedLogin" if vm.o(t).class.isa("MordhauTitleScreen") => r(V::None),
        // UMordhauTitleScreen::ConsumeKeyPressed 0x1618eb0 (ConsumeMouseKeyEvent 0x1619470 forwards mouse buttons):
        // both keys must be valid; a mouse button, KeyToContinue or GamepadKeyToContinue signs in through
        // HandleLoginUIClosed 0x162b2f0 -> UMordhauGameInstance::OnNewUserSignIn, OnUserConfirmedLogin, then the
        // widget's vtable +0x278 call with 1 (SetVisibility(Collapsed)); returns true. The online identity / licence
        // queries are taken as passing (a signed-in PC: UNCONFIRMED), and the sign-in as pairing the user
        // (UMordhauGameInstance::SetUserControllerPairing 0x156c550; who calls it is UNCONFIRMED)
        "ConsumeKeyPressed" | "ConsumeMouseKeyEvent" if vm.o(t).class.isa("MordhauTitleScreen") => {
            let key = if name == "ConsumeMouseKeyEvent" { "LeftMouseButton".to_string() } else { a1.field("Key").field("KeyName").s() };
            let k1 = vm.prop(t, "KeyToContinue").field("KeyName").s();
            let k2 = vm.prop(t, "GamepadKeyToContinue").field("KeyName").s();
            let valid = |k: &str| !k.is_empty() && k != "None";
            if !valid(&k1) || !valid(&k2) {
                return r(V::Bool(false));
            }
            if key.ends_with("MouseButton") || key == k1 || key == k2 {
                if let Some(gi) = vm.world.get("gi").copied() {
                    vm.set(gi, "bIsUserControllerPaired", V::Bool(true));
                }
                vm.event(t, "OnUserConfirmedLogin", vec![]);
                widget_method(vm, t, "", "SetVisibility", &[V::Int(1)]);
                return r(V::Bool(true));
            }
            r(V::Bool(false))
        }
        "IsUserControllerPaired" => r(V::Bool(vm.prop(t, "bIsUserControllerPaired").truthy())),
        "SetUserControllerPairing" => set(vm, "bIsUserControllerPaired", V::Bool(a0.truthy())),
        // parent-class versions of Blueprint events (UUserWidget's native Construct / PreConstruct / Tick ... are
        // empty BlueprintImplementableEvents)
        "Construct" | "PreConstruct" | "Destruct" | "Tick" | "OnInitialized" | "ReceiveBeginPlay" | "ReceiveTick" => r(V::None),
        // `UMordhauNewsWidget::SendHTTPRequest` rva 0x14d2c60 / OnRequestComplete rva 0x14ce970: news.rs (the request
        // is answered from the rewrite's local news document; completion arrives on a later frame, news::poll)
        "SendHTTPRequest" if vm.o(t).class.isa("MordhauNewsWidget") => r(V::Bool(crate::news::send_http_request(vm, t))),
        // UAsyncTaskDownloadImage::DownloadImage (UE 4.26): the task starts at creation; Activate is
        // UBlueprintAsyncActionBase's empty default
        "DownloadImage" => r(crate::news::download_image(vm, &a0.s())),
        "Activate" if vm.alive(t) && vm.o(t).class.native() == "AsyncTaskDownloadImage" => r(V::None),
        // URichTextBlock::SetTextStyleSet (UE 4.26 RichTextBlock.cpp): the style table the runs read
        "SetTextStyleSet" => set(vm, "TextStyleSet", a0),
        // UTextBlock / URichTextBlock
        "SetText" => set(vm, "Text", V::Text(a0.s())),
        "GetText" if native == "TextBlock" || native == "RichTextBlock" || native == "EditableTextBox" || native == "EditableText" || native == "MultiLineEditableTextBox" => r(V::Text(vm.prop(t, "Text").s())),
        "SetShadowColorAndOpacity" => set(vm, "ShadowColorAndOpacity", a0),
        "SetShadowOffset" => set(vm, "ShadowOffset", a0),
        "SetFont" => set(vm, "Font", a0),
        "SetJustification" => set(vm, "Justification", a0),
        "SetAutoWrapText" => set(vm, "AutoWrapText", a0),
        "SetMinDesiredWidth" if native == "TextBlock" => set(vm, "MinDesiredWidth", a0),
        "SetOpacity" => {
            let k = if native == "TextBlock" { "ColorAndOpacity" } else { "ColorAndOpacity" };
            let mut c = color_of(&vm.prop(t, k), [1.0; 4]);
            c[3] = a0.f();
            let v = if native == "TextBlock" { V::st(&[("SpecifiedColor", lin(c)), ("ColorUseRule", V::Int(0))]) } else { lin(c) };
            set(vm, k, v)
        }
        "SetColorAndOpacity" => set(vm, "ColorAndOpacity", a0),
        // UImage
        "SetBrush" if native == "Border" => set(vm, "Background", a0),
        "SetBrush" => set(vm, "Brush", a0),
        "SetBrushFromTexture" | "SetBrushFromTextureDynamic" | "SetBrushFromMaterial" | "SetBrushFromAsset" | "SetBrushResourceObject" | "SetBrushFromSoftTexture" | "SetBrushFromSoftMaterial" => {
            let k = if native == "Border" { "Background" } else { "Brush" };
            let mut b = vm.prop(t, k);
            if !matches!(b, V::Struct(_)) {
                b = V::Struct(Box::default());
            }
            let matchsz = a.get(1).map(V::truthy).unwrap_or(false);
            if let V::Struct(m) = &mut b {
                m.insert("ResourceObject".into(), a0.clone());
                if matchsz {
                    if let V::Asset(x) = &a0 {
                        if let Some((w, h)) = vm.src.as_ref().and_then(|s| crate::slate::texture_size(s, &x.package)) {
                            m.insert("ImageSize".into(), V::st(&[("X", V::Float(w)), ("Y", V::Float(h))]));
                        }
                    }
                }
            }
            set(vm, k, b)
        }
        "SetBrushTintColor" => setf(vm, &["Brush", "TintColor"], a0),
        "SetBrushSize" => setf(vm, &["Brush", "ImageSize"], a0),
        "GetDynamicMaterial" | "GetDynamicMaterialInstance" => {
            // UImage::GetDynamicMaterial: the brush's material made dynamic (created on first use)
            let b = vm.prop(t, "Brush");
            match b.field("ResourceObject") {
                V::Obj(m) => r(V::Obj(*m)),
                parent => {
                    let parent = parent.clone();
                    let c = vm.native_class("MaterialInstanceDynamic");
                    let id = vm.new_obj(c, "MID");
                    vm.set(id, "Parent", parent);
                    let mut b2 = b.clone();
                    if let V::Struct(m) = &mut b2 {
                        m.insert("ResourceObject".into(), V::Obj(id));
                    }
                    vm.set(t, "Brush", b2);
                    r(V::Obj(id))
                }
            }
        }
        // UMaterialInstanceDynamic parameters
        "SetScalarParameterValue" | "SetVectorParameterValue" | "SetTextureParameterValue" if vm.o(t).class.name == "MaterialInstanceDynamic" => {
            let k = format!("param:{}", a0.s());
            set(vm, &k, a1)
        }
        "K2_GetScalarParameterValue" | "K2_GetVectorParameterValue" | "K2_GetTextureParameterValue" if vm.o(t).class.name == "MaterialInstanceDynamic" => r(vm.prop(t, &format!("param:{}", a0.s()))),
        // UButton
        "SetStyle" => set(vm, "WidgetStyle", a0),
        "SetBackgroundColor" => set(vm, "BackgroundColor", a0),
        "SetClickMethod" | "SetTouchMethod" | "SetPressMethod" => r(V::None),
        // UBorder
        "SetBrushColor" => set(vm, "BrushColor", a0),
        "SetContentColorAndOpacity" => set(vm, "ContentColorAndOpacity", a0),
        "SetDesiredSizeScale" => set(vm, "DesiredSizeScale", a0),
        // UProgressBar
        "SetPercent" => set(vm, "Percent", V::Float(a0.f())),
        "SetFillColorAndOpacity" => set(vm, "FillColorAndOpacity", a0),
        "SetIsMarquee" => set(vm, "bIsMarquee", a0),
        // UCheckBox
        "SetIsChecked" => set(vm, "CheckedState", V::Int(if a0.truthy() { 1 } else { 0 })),
        "IsChecked" => r(V::Bool(vm.prop(t, "CheckedState").i() == 1)),
        "SetCheckedState" => set(vm, "CheckedState", V::Int(a0.i())),
        "GetCheckedState" => r(V::Int(vm.prop(t, "CheckedState").i())),
        // USlider / USpinBox
        "SetValue" if native == "Slider" || native == "SpinBox" => set(vm, "Value", V::Float(a0.f())),
        "GetValue" if native == "Slider" || native == "SpinBox" => r(V::Float(vm.prop(t, "Value").f())),
        // UComboBoxString
        "AddOption" => {
            let mut o = vm.prop(t, "DefaultOptions").arr().to_vec();
            o.push(V::Str(a0.s()));
            set(vm, "DefaultOptions", V::Array(o))
        }
        "ClearOptions" => set(vm, "DefaultOptions", V::Array(vec![])),
        "GetOptionCount" => r(V::Int(vm.prop(t, "DefaultOptions").arr().len() as i64)),
        "GetOptionAtIndex" => r(V::Str(vm.prop(t, "DefaultOptions").arr().get(a0.i().max(0) as usize).map(V::s).unwrap_or_default())),
        // UComboBoxString::SetSelectedOption / SetSelectedIndex -> SComboBox::SetSelectedItem -> OnSelectionChanged
        // (SelectedItem, ESelectInfo::Direct 3) when the selection changes (UE 4.26 ComboBoxString.cpp)
        "SetSelectedOption" | "SetSelectedIndex" => {
            let s = if name == "SetSelectedIndex" { vm.prop(t, "DefaultOptions").arr().get(a0.i().max(0) as usize).map(V::s).unwrap_or_default() } else { a0.s() };
            let old = vm.prop(t, "SelectedOption").s();
            vm.set(t, "SelectedOption", V::Str(s.clone()));
            if old != s || !vm.prop(t, "__selected_once").truthy() {
                vm.set(t, "__selected_once", V::Bool(true));
                if let V::Multi(m) = vm.prop(t, "OnSelectionChanged") {
                    for (o, f) in m {
                        vm.call_named(o, &f, vec![V::Str(s.clone()), V::Int(3)]);
                    }
                }
            }
            r(V::None)
        }
        "GetSelectedOption" => r(V::Str(vm.prop(t, "SelectedOption").s())),
        "GetSelectedIndex" => {
            let s = vm.prop(t, "SelectedOption").s();
            r(V::Int(vm.prop(t, "DefaultOptions").arr().iter().position(|x| x.s() == s).map_or(-1, |p| p as i64)))
        }
        "FindOptionIndex" => {
            let s = a0.s();
            r(V::Int(vm.prop(t, "DefaultOptions").arr().iter().position(|x| x.s() == s).map_or(-1, |p| p as i64)))
        }
        // UWidgetSwitcher
        "SetActiveWidgetIndex" => {
            let n = vm.o(t).slots.len() as i64;
            // UWidgetSwitcher::SetActiveWidgetIndex 0x2c18010 -> SWidgetSwitcher::SetActiveWidgetIndex: clamped to the
            // valid range (UNCONFIRMED: the clamp is the 4.26 source's)
            set(vm, "ActiveWidgetIndex", V::Int(a0.i().clamp(0, (n - 1).max(0))))
        }
        "GetActiveWidgetIndex" => r(V::Int(vm.prop(t, "ActiveWidgetIndex").i())),
        "GetNumWidgets" => r(V::Int(vm.o(t).slots.len() as i64)),
        "GetWidgetAtIndex" => r(children(vm, t).get(a0.i().max(0) as usize).map(|&w| V::Obj(w)).unwrap_or_default()),
        "SetActiveWidget" => {
            let k = children(vm, t).iter().position(|&w| Some(w) == a0.obj());
            if let Some(k) = k {
                vm.set(t, "ActiveWidgetIndex", V::Int(k as i64));
            }
            r(V::None)
        }
        "GetActiveWidget" => {
            let i = vm.prop(t, "ActiveWidgetIndex").i().max(0) as usize;
            r(children(vm, t).get(i).map(|&w| V::Obj(w)).unwrap_or_default())
        }
        // UPanelWidget
        "AddChild" | "AddChildToVerticalBox" | "AddChildToHorizontalBox" | "AddChildToOverlay" | "AddChildToCanvas" | "AddChildToUniformGrid" | "AddChildToGrid" | "AddChildToWrapBox" | "AddChildToBorder" | "AddChildToSizeBox" => match a0.obj() {
            Some(c) => r(add_child(vm, t, c)),
            None => r(V::None),
        },
        "SetContent" => match a0.obj() {
            Some(c) => r(add_child(vm, t, c)),
            None => r(V::None),
        },
        "GetContent" => r(children(vm, t).first().map(|&w| V::Obj(w)).unwrap_or_default()),
        "InsertChildAt" => match a1.obj() {
            Some(c) => {
                let s = add_child(vm, t, c);
                if let V::Obj(sid) = s {
                    let i = (a0.i().max(0) as usize).min(vm.o(t).slots.len() - 1);
                    vm.om(t).slots.retain(|&x| x != sid);
                    vm.om(t).slots.insert(i, sid);
                }
                r(s)
            }
            None => r(V::None),
        },
        "ClearChildren" => {
            for c in children(vm, t) {
                vm.om(c).parent_slot = None;
            }
            vm.om(t).slots.clear();
            r(V::None)
        }
        "GetChildAt" => r(children(vm, t).get(a0.i().max(0) as usize).map(|&w| V::Obj(w)).unwrap_or_default()),
        "GetChildrenCount" => r(V::Int(vm.o(t).slots.len() as i64)),
        "GetAllChildren" => r(V::Array(children(vm, t).into_iter().map(V::Obj).collect())),
        "HasAnyChildren" => r(V::Bool(!vm.o(t).slots.is_empty())),
        "HasChild" => r(V::Bool(a0.obj().is_some_and(|c| children(vm, t).contains(&c)))),
        "GetChildIndex" => r(V::Int(a0.obj().and_then(|c| children(vm, t).iter().position(|&x| x == c)).map_or(-1, |p| p as i64))),
        "RemoveChild" => match a0.obj() {
            Some(c) if children(vm, t).contains(&c) => {
                remove_from_parent(vm, c);
                r(V::Bool(true))
            }
            _ => r(V::Bool(false)),
        },
        "RemoveChildAt" => match children(vm, t).get(a0.i().max(0) as usize).copied() {
            Some(c) => {
                remove_from_parent(vm, c);
                r(V::Bool(true))
            }
            None => r(V::Bool(false)),
        },
        // UListView / UTileView
        "AddItem" if is_list(vm, t) => {
            list_add(vm, t, a0.clone());
            r(V::None)
        }
        "RemoveItem" if is_list(vm, t) => {
            let items = vm.prop(t, "ListItems").arr().to_vec();
            if let Some(i) = items.iter().position(|x| x.same(&a0)) {
                list_remove_at(vm, t, i);
            }
            r(V::None)
        }
        "ClearListItems" if is_list(vm, t) => {
            let n = vm.prop(t, "ListItems").arr().len();
            for i in (0..n).rev() {
                list_remove_at(vm, t, i);
            }
            r(V::None)
        }
        "SetListItems" if is_list(vm, t) => {
            let n = vm.prop(t, "ListItems").arr().len();
            for i in (0..n).rev() {
                list_remove_at(vm, t, i);
            }
            for x in a0.arr().to_vec() {
                list_add(vm, t, x);
            }
            r(V::None)
        }
        "GetNumItems" if is_list(vm, t) => r(V::Int(vm.prop(t, "ListItems").arr().len() as i64)),
        "GetItemAt" if is_list(vm, t) => r(vm.prop(t, "ListItems").arr().get(a0.i().max(0) as usize).cloned().unwrap_or_default()),
        "GetListItems" if is_list(vm, t) => r(vm.prop(t, "ListItems")),
        "GetIndexForItem" if is_list(vm, t) => r(V::Int(vm.prop(t, "ListItems").arr().iter().position(|x| x.same(&a0)).map_or(-1, |p| p as i64))),
        "GetDisplayedEntryWidgets" if is_list(vm, t) => r(vm.prop(t, "__entries")),
        "BP_GetSelectedItem" if is_list(vm, t) => r(vm.prop(t, "__selected")),
        "BP_SetSelectedItem" | "SetSelectedItem" if is_list(vm, t) => set(vm, "__selected", a0),
        "BP_ClearSelection" | "ClearSelection" if is_list(vm, t) => set(vm, "__selected", V::None),
        "ScrollToTop" if is_list(vm, t) => set(vm, "__scroll", V::Float(0.0)),
        "ScrollToBottom" if is_list(vm, t) => set(vm, "__scroll", V::Float(f64::MAX)),
        "RequestRefresh" | "RegenerateAllEntries" | "NavigateToIndex" | "ScrollIndexIntoView" | "BP_ScrollItemIntoView" | "BP_NavigateToItem" | "SetSelectionMode" if is_list(vm, t) => r(V::None),
        // UScrollBox
        "ScrollToEnd" => set(vm, "__scroll", V::Float(f64::MAX)),
        "ScrollToStart" => set(vm, "__scroll", V::Float(0.0)),
        "SetScrollOffset" => set(vm, "__scroll", V::Float(a0.f())),
        "GetScrollOffset" => r(V::Float(vm.prop(t, "__scroll").f())),
        "ScrollWidgetIntoView" | "EndInertialScrolling" | "SetScrollBarVisibility" | "SetAlwaysShowScrollbar" => r(V::None),
        // USizeBox
        "SetWidthOverride" => {
            vm.set(t, "bOverride_WidthOverride", V::Bool(true));
            set(vm, "WidthOverride", V::Float(a0.f()))
        }
        "SetHeightOverride" => {
            vm.set(t, "bOverride_HeightOverride", V::Bool(true));
            set(vm, "HeightOverride", V::Float(a0.f()))
        }
        "SetMinDesiredWidth" | "SetMinDesiredHeight" | "SetMaxDesiredWidth" | "SetMaxDesiredHeight" | "SetMinAspectRatio" | "SetMaxAspectRatio" => {
            let k = &name[3..];
            vm.set(t, &format!("bOverride_{k}"), V::Bool(true));
            set(vm, k, V::Float(a0.f()))
        }
        "ClearWidthOverride" | "ClearHeightOverride" | "ClearMinDesiredWidth" | "ClearMinDesiredHeight" | "ClearMaxDesiredWidth" | "ClearMaxDesiredHeight" => {
            let k = &name[5..];
            set(vm, &format!("bOverride_{k}"), V::Bool(false))
        }
        // UScaleBox
        "SetStretch" => set(vm, "Stretch", a0),
        "SetUserSpecifiedScale" => set(vm, "UserSpecifiedScale", a0),
        // slots
        "SetPadding" => set(vm, "Padding", a0),
        "SetHorizontalAlignment" => set(vm, "HorizontalAlignment", V::Int(a0.i())),
        "SetVerticalAlignment" => set(vm, "VerticalAlignment", V::Int(a0.i())),
        "SetSize" if native.ends_with("BoxSlot") => set(vm, "Size", a0),
        "SetSize" if native == "CanvasPanelSlot" => {
            let s = v2_of(&a0, [0.0; 2]);
            let mut ld = vm.prop(t, "LayoutData");
            let mut off = ld.field("Offsets").clone();
            if !matches!(off, V::Struct(_)) {
                off = V::Struct(Box::default());
            }
            if let V::Struct(m) = &mut off {
                m.insert("Right".into(), V::Float(s[0]));
                m.insert("Bottom".into(), V::Float(s[1]));
            }
            if !matches!(ld, V::Struct(_)) {
                ld = V::Struct(Box::default());
            }
            if let V::Struct(m) = &mut ld {
                m.insert("Offsets".into(), off);
            }
            set(vm, "LayoutData", ld)
        }
        "SetPosition" if native == "CanvasPanelSlot" => {
            let s = v2_of(&a0, [0.0; 2]);
            let mut ld = vm.prop(t, "LayoutData");
            if !matches!(ld, V::Struct(_)) {
                ld = V::Struct(Box::default());
            }
            if let V::Struct(m) = &mut ld {
                let off = m.entry("Offsets".into()).or_insert_with(|| V::Struct(Box::default()));
                if !matches!(off, V::Struct(_)) {
                    *off = V::Struct(Box::default());
                }
                if let V::Struct(o) = off {
                    o.insert("Left".into(), V::Float(s[0]));
                    o.insert("Top".into(), V::Float(s[1]));
                }
            }
            set(vm, "LayoutData", ld)
        }
        "GetPosition" if native == "CanvasPanelSlot" => {
            let ld = vm.prop(t, "LayoutData");
            r(V::st(&[("X", V::Float(ld.field("Offsets").field("Left").f())), ("Y", V::Float(ld.field("Offsets").field("Top").f()))]))
        }
        "GetSize" if native == "CanvasPanelSlot" => {
            let ld = vm.prop(t, "LayoutData");
            r(V::st(&[("X", V::Float(ld.field("Offsets").field("Right").f())), ("Y", V::Float(ld.field("Offsets").field("Bottom").f()))]))
        }
        "SetZOrder" => set(vm, "ZOrder", V::Int(a0.i())),
        "SetAutoSize" => set(vm, "bAutoSize", V::Bool(a0.truthy())),
        "SetAnchors" => setf(vm, &["LayoutData", "Anchors"], a0),
        "SetAlignment" => setf(vm, &["LayoutData", "Alignment"], a0),
        "SetOffsets" => setf(vm, &["LayoutData", "Offsets"], a0),
        "SetLayout" => set(vm, "LayoutData", a0),
        "SetColumn" => set(vm, "Column", V::Int(a0.i())),
        "SetRow" => set(vm, "Row", V::Int(a0.i())),
        "SetColumnSpan" => set(vm, "ColumnSpan", V::Int(a0.i())),
        "SetRowSpan" => set(vm, "RowSpan", V::Int(a0.i())),
        // UEditableTextBox
        "SetHintText" => set(vm, "HintText", a0),
        "SetIsReadOnly" => set(vm, "IsReadOnly", a0),
        "SetError" | "ClearError" => r(V::None),
        _ => proxy(vm, t, name, a),
    }
}

/// is `w` reachable in a visible part of the viewport (UWidget::IsVisible checks the widget's own Slate visibility;
/// here also that it is attached)
fn is_shown(vm: &Vm, w: crate::model::Id) -> bool {
    let _ = (vm, w);
    true
}

/// Mordhau / engine objects the host stands in for (settings, player controller, HUD actor, game state): generic
/// property access by accessor name (UNCONFIRMED: the native getters are assumed to return the same-named property;
/// host.rs fills the values the HUD and menus read)
fn proxy(vm: &mut Vm, t: crate::model::Id, name: &str, a: &[V]) -> Option<NRet> {
    if !vm.alive(t) {
        return None;
    }
    if vm.world.get("input") == Some(&t) {
        if let Some((ret, o)) = crate::input::call(vm, t, name, a) {
            return outs(ret, o);
        }
    }
    if vm.world.get("settings") == Some(&t) {
        if let Some(v) = crate::settings::call(vm, t, name, a) {
            // `GetAvailableLanguages` 0x1594de0 copies into its out parameter (argument 0)
            if name == "GetAvailableLanguages" {
                return outs(V::None, vec![(0, v)]);
            }
            return r(v);
        }
    }
    let stateful = { let c = &vm.o(t).class; c.isa("PlayerState") || c.isa("GameStateBase") || c.isa("GameState") || c.isa("MordhauPlayerState") };
    if !vm.o(t).class.package.is_empty() && !vm.world.values().any(|&w| w == t) && !stateful {
        return None;
    }
    if let Some(k) = name.strip_prefix("Get") {
        if k.ends_with("Limits") {
            // UMordhauGameUserSettings::Get*Limits(out Min, out Max) (UNCONFIRMED values: host fills "<X>Limits")
            let l = vm.prop(t, k);
            return outs(V::None, vec![(0, l.field("Min").clone()), (1, l.field("Max").clone())]);
        }
        let v = vm.prop(t, k);
        if !matches!(v, V::None) {
            return r(v);
        }
        return r(vm.prop(t, &format!("b{k}")));
    }
    if let Some(k) = name.strip_prefix("Set") {
        if let Some(v) = a.first() {
            vm.set(t, k, v.clone());
            return r(V::None);
        }
    }
    if name.starts_with("Is") || name.starts_with("Are") || name.starts_with("Can") || name.starts_with("Has") || name.starts_with("Should") {
        return r(V::Bool(vm.prop(t, name).truthy() || vm.prop(t, &format!("b{}", name.trim_start_matches("Is"))).truthy()));
    }
    None
}

/// a UEnum entry's name (or UserDefinedEnum display name) by value
fn enum_name(vm: &mut Vm, e: &V, value: i64, friendly: bool) -> String {
    let V::Asset(a) = e else { return value.to_string() };
    if a.package.is_empty() || a.package.starts_with("/Script/") {
        return value.to_string();
    }
    let Some(ex) = vm.rd.read(&a.package) else { return value.to_string() };
    for x in &ex {
        let Some(p) = x.get("Properties") else { continue };
        let names = x.get("Names").or_else(|| p.get("Names")).and_then(serde_json::Value::as_array);
        if let Some(names) = names {
            // Names: [[ "E_X::NewEnumeratorN", value ], ...] (CUE4Parse UEnum json)
            let mut key = String::new();
            for n in names {
                let (k, v) = match n {
                    serde_json::Value::Array(kv) if kv.len() == 2 => (kv[0].as_str().unwrap_or(""), kv[1].as_i64().unwrap_or(-1)),
                    serde_json::Value::Object(o) => (o.get("Key").and_then(|k| k.as_str()).unwrap_or(""), o.get("Value").and_then(|v| v.as_i64()).unwrap_or(-1)),
                    _ => continue,
                };
                if v == value {
                    key = k.to_string();
                }
            }
            if friendly {
                for d in p.get("DisplayNameMap").and_then(serde_json::Value::as_array).into_iter().flatten() {
                    let k = d.get("Key").and_then(serde_json::Value::as_str).unwrap_or("");
                    if !key.is_empty() && key.ends_with(k) {
                        return mh_assets::umg::text(d.get("Value"));
                    }
                }
            }
            if !key.is_empty() {
                return key.rsplit("::").next().unwrap_or(&key).to_string();
            }
        }
    }
    value.to_string()
}

/// an FKey's name; a default-constructed FKey (an invalid-index Array_Get result, UKismetArrayLibrary
/// GenericArray_Get leaves the item default) is NAME_None
fn key_name(k: &V) -> String {
    let n = k.field("KeyName").s();
    if n.is_empty() {
        "None".into()
    } else {
        n
    }
}

fn is_list(vm: &Vm, t: crate::model::Id) -> bool {
    matches!(vm.o(t).class.native(), "ListView" | "TileView" | "TreeView" | "ListViewBase")
}

/// add an item and its entry widget (EntryWidgetClass), which gets OnListItemObjectSet(item)
fn list_add(vm: &mut Vm, t: crate::model::Id, item: V) {
    let mut items = vm.prop(t, "ListItems").arr().to_vec();
    items.push(item.clone());
    vm.set(t, "ListItems", V::Array(items));
    let mut entries = vm.prop(t, "__entries").arr().to_vec();
    let entry = match vm.prop(t, "EntryWidgetClass") {
        V::Asset(c) => {
            let cls = vm.class(&c);
            let n = c.name.trim_end_matches("_C").to_string();
            let e = vm.create(cls, &n);
            vm.construct(e);
            vm.event(e, "OnListItemObjectSet", vec![item.clone()]);
            // UListView OnEntryInitialized(Item, Widget)
            if let V::Multi(m) = vm.prop(t, "BP_OnEntryInitialized") {
                for (o, f) in m {
                    vm.call_named(o, &f, vec![item.clone(), V::Obj(e)]);
                }
            }
            V::Obj(e)
        }
        _ => V::None,
    };
    entries.push(entry);
    vm.set(t, "__entries", V::Array(entries));
}

fn list_remove_at(vm: &mut Vm, t: crate::model::Id, i: usize) {
    let mut items = vm.prop(t, "ListItems").arr().to_vec();
    let mut entries = vm.prop(t, "__entries").arr().to_vec();
    if i < items.len() {
        items.remove(i);
    }
    if i < entries.len() {
        if let V::Obj(e) = entries.remove(i) {
            vm.event(e, "BP_OnEntryReleased", vec![]);
        }
    }
    vm.set(t, "ListItems", V::Array(items));
    vm.set(t, "__entries", V::Array(entries));
}

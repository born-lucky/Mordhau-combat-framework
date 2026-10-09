//! UMG widget animations (UWidgetAnimation -> UMovieScene) evaluated the way Sequencer does for the property tracks
//! UMG uses: float (MovieSceneFloatTrack: RenderOpacity, Percent, ...), colour (MovieSceneColorTrack: R/G/B/A
//! channels of a FLinearColor / FSlateColor property), 2D transform (MovieScene2DTransformTrack: RenderTransform
//! Translation / Rotation / Scale / Shear) and margin (MovieSceneMarginTrack).
//!
//! The channel keys are FMovieSceneFloatChannel, a native struct mh-pak leaves undecoded; it is read here from the
//! section export's raw tagged-property bytes, layout as UE 4.26 FMovieSceneFloatChannel::Serialize (CUE4Parse
//! FMovieSceneFloatChannel): PreInfinityExtrap u8, PostInfinityExtrap u8, Times (i32 element size, i32 count, count x
//! FFrameNumber i32), Values (i32 element size, i32 count, count x FMovieSceneFloatValue: Value f32, Tangent {Arrive
//! f32, Leave f32, ArriveWeight f32, LeaveWeight f32, WeightMode u8 + 3 pad}, InterpMode u8, TangentMode u8, 2 pad),
//! DefaultValue f32, bHasDefaultValue (u32 bool), TickResolution FFrameRate (i32 x2).
//! Evaluation (MovieSceneFloatChannel.cpp FMovieSceneFloatChannel::Evaluate, UE 4.26): before the first / after the last
//! key the key's value (constant extrapolation, RCCE_Constant: the cooked UI curves use it, UNCONFIRMED for others);
//! between keys by the left key's ERichCurveInterpMode: Linear 0 = lerp, Constant 1 = left value, Cubic 2 = Bezier
//! P1 = P0 + Leave x dt / 3, P2 = P3 - Arrive x dt / 3 (dt in ticks; weighted tangents not applied: UNCONFIRMED);
//! no keys -> DefaultValue when bHasDefaultValue.

use crate::model::*;
use crate::vm::Vm;
use mh_pak::asset::Package;
use mh_pak::Cursor;
use std::collections::HashMap;
use std::rc::Rc;

#[derive(Clone, Debug, Default)]
pub struct Channel {
    pub times: Vec<i32>,
    /// (value, arrive tangent, leave tangent, interp mode)
    pub keys: Vec<(f32, f32, f32, u8)>,
    pub default: Option<f32>,
}

impl Channel {
    pub fn eval(&self, tick: f64) -> Option<f64> {
        if self.times.is_empty() || self.keys.len() != self.times.len() {
            return self.default.map(|d| d as f64);
        }
        let n = self.times.len();
        if tick <= self.times[0] as f64 {
            return Some(self.keys[0].0 as f64);
        }
        if tick >= self.times[n - 1] as f64 {
            return Some(self.keys[n - 1].0 as f64);
        }
        let i = self.times.windows(2).position(|w| tick >= w[0] as f64 && tick < w[1] as f64).unwrap_or(n - 2);
        let (t0, t1) = (self.times[i] as f64, self.times[i + 1] as f64);
        let (v0, _, leave, mode) = self.keys[i];
        let (v1, arrive, _, _) = self.keys[i + 1];
        let dt = t1 - t0;
        let a = if dt > 0.0 { (tick - t0) / dt } else { 0.0 };
        Some(match mode {
            1 => v0 as f64,
            2 => {
                let (p0, p3) = (v0 as f64, v1 as f64);
                let p1 = p0 + leave as f64 * dt / 3.0;
                let p2 = p3 - arrive as f64 * dt / 3.0;
                let u = 1.0 - a;
                u * u * u * p0 + 3.0 * u * u * a * p1 + 3.0 * u * a * a * p2 + a * a * a * p3
            }
            _ => v0 as f64 + (v1 as f64 - v0 as f64) * a,
        })
    }
}

fn read_channel(r: &mut Cursor) -> Option<Channel> {
    r.u8();
    r.u8();
    let _es = r.s32();
    let n = r.s32();
    if !(0..100_000).contains(&n) {
        return None;
    }
    let times: Vec<i32> = (0..n).map(|_| r.s32()).collect();
    let es = r.s32();
    let m = r.s32();
    if !(0..100_000).contains(&m) || !(20..=64).contains(&es) {
        return None;
    }
    let mut keys = vec![];
    for _ in 0..m {
        let start = r.p;
        let v = r.f32raw();
        let arrive = r.f32raw();
        let leave = r.f32raw();
        r.p = start + 24; // Value + Tangent (16 + weight mode / pad)
        let interp = r.u8();
        r.p = start + es as i64;
        keys.push((v, arrive, leave, interp));
    }
    let def = r.f32raw();
    let has = r.s32() != 0;
    if r.bad {
        return None;
    }
    Some(Channel { times, keys, default: has.then_some(def) })
}

/// the channel-valued properties of an export (name -> channel), from its raw tagged property stream
fn export_channels(pk: &Package, ei: usize) -> HashMap<String, Channel> {
    let mut out = HashMap::new();
    let ex = &pk.exports[ei];
    let mut r = pk.cursor();
    r.p = ex.off;
    let end = ex.off + ex.size;
    while r.p + 8 <= end {
        let name = pk.fname(&mut r);
        if name == "None" {
            break;
        }
        let typ = pk.fname(&mut r);
        let size = r.s32();
        let aidx = r.s32();
        let mut sname = String::new();
        match typ.as_str() {
            "StructProperty" => {
                sname = pk.fname(&mut r);
                r.skip(16);
            }
            "BoolProperty" => {
                r.u8();
            }
            "ByteProperty" | "EnumProperty" | "ArrayProperty" | "SetProperty" => {
                pk.fname(&mut r);
            }
            "MapProperty" => {
                pk.fname(&mut r);
                pk.fname(&mut r);
            }
            _ => {}
        }
        if r.u8() != 0 {
            r.skip(16);
        }
        let start = r.p;
        if r.bad || size < 0 || start + size as i64 > end {
            break;
        }
        if sname == "MovieSceneFloatChannel" {
            let mut c = pk.cursor();
            c.p = start;
            if let Some(ch) = read_channel(&mut c) {
                let key = if aidx == 0 { name } else { format!("{name}[{aidx}]") };
                out.insert(key, ch);
            }
        }
        r.p = start + size as i64;
    }
    out
}

/// One property track of an animation
#[derive(Clone, Debug)]
pub struct Track {
    /// the animated widget's variable name ("" = the user widget's root)
    pub widget: String,
    /// "Float" | "Color" | "Transform" | "Margin"
    pub kind: String,
    /// the bound property path ("RenderOpacity", "ContentColorAndOpacity", "RenderTransform", ...)
    pub property: String,
    /// channels by name within the section ("FloatCurve", "RedCurve", "Translation", "Translation[1]", ...)
    pub channels: HashMap<String, Channel>,
    /// section start / end in ticks
    pub range: (i64, i64),
    /// EMovieSceneCompletionMode::RestoreState: values reset after the animation (KeepState otherwise)
    pub restore: bool,
}

#[derive(Clone, Debug, Default)]
pub struct Anim {
    pub tick_resolution: f64,
    pub tracks: Vec<Track>,
}

/// every widget animation of a widget Blueprint package: name (without _INST) -> tracks
pub fn read_package(rd: &mh_pak::Reader, pkg: &str) -> HashMap<String, Rc<Anim>> {
    let mut out = HashMap::new();
    let (Some(pk), Some(ex)) = (rd.open(pkg), rd.read(pkg)) else { return out };
    let idx = |v: Option<&serde_json::Value>| -> Option<usize> { v?.get("ObjectPath")?.as_str()?.rsplit_once('.')?.1.parse().ok() };
    for e in &ex {
        if e.get("Type").and_then(|t| t.as_str()) != Some("WidgetAnimation") {
            continue;
        }
        let name = e.get("Name").and_then(|n| n.as_str()).unwrap_or("").trim_end_matches("_INST").to_string();
        let p = &e["Properties"];
        let guid_widget: HashMap<String, String> = p["AnimationBindings"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|b| {
                let w = if b["bIsRootWidget"].as_bool() == Some(true) { String::new() } else { b["WidgetName"].as_str().unwrap_or("").to_string() };
                (b["AnimationGuid"].as_str().unwrap_or("").to_string(), w)
            })
            .collect();
        let Some(ms) = idx(p.get("MovieScene")).and_then(|i| ex.get(i)) else { continue };
        let mp = &ms["Properties"];
        let tick_resolution = mp.pointer("/TickResolution/Numerator").and_then(|x| x.as_f64()).unwrap_or(60000.0);
        let mut tracks = vec![];
        for b in mp["ObjectBindings"].as_array().into_iter().flatten() {
            let Some(widget) = guid_widget.get(b["ObjectGuid"].as_str().unwrap_or("")) else { continue };
            for t in b["Tracks"].as_array().into_iter().flatten() {
                let Some(ti) = idx(Some(t)) else { continue };
                let te = &ex[ti];
                let tt = te["Type"].as_str().unwrap_or("");
                let kind = match tt {
                    "MovieSceneFloatTrack" => "Float",
                    "MovieSceneColorTrack" => "Color",
                    "MovieScene2DTransformTrack" => "Transform",
                    "MovieSceneMarginTrack" => "Margin",
                    _ => continue,
                };
                let property = te.pointer("/Properties/PropertyBinding/PropertyPath").and_then(|x| x.as_str()).unwrap_or("").to_string();
                for s in te.pointer("/Properties/Sections").and_then(|x| x.as_array()).into_iter().flatten() {
                    let Some(si) = idx(Some(s)) else { continue };
                    let sp = &ex[si]["Properties"];
                    let lo = sp.pointer("/SectionRange/Value/LowerBound/Value/Value").and_then(|x| x.as_i64()).unwrap_or(i64::MIN);
                    let hi = sp.pointer("/SectionRange/Value/UpperBound/Value/Value").and_then(|x| x.as_i64()).unwrap_or(i64::MAX);
                    let restore = sp.pointer("/EvalOptions/CompletionMode").and_then(|x| x.as_str()).is_some_and(|m| m.ends_with("RestoreState"));
                    let channels = export_channels(&pk, si);
                    tracks.push(Track { widget: widget.clone(), kind: kind.to_string(), property: property.clone(), channels, range: (lo, hi), restore });
                }
            }
        }
        out.insert(name, Rc::new(Anim { tick_resolution, tracks }));
    }
    out
}

/// apply an animation at `seconds` into it to the user widget `owner`'s widgets
pub fn apply(vm: &mut Vm, owner: Id, anim: &Anim, seconds: f64) {
    let tick = seconds * anim.tick_resolution;
    for t in &anim.tracks {
        let target = if t.widget.is_empty() { vm.o(owner).root } else { vm.child(owner, &t.widget) };
        let Some(w) = target else { continue };
        let tk = (tick as i64).clamp(t.range.0, t.range.1) as f64;
        let ch = |n: &str| t.channels.get(n).and_then(|c| c.eval(tk));
        match t.kind.as_str() {
            "Float" => {
                if let Some(v) = ch("FloatCurve") {
                    vm.set(w, &t.property, V::Float(v));
                }
            }
            "Color" => {
                let cur = vm.prop(w, &t.property);
                let slate = matches!(cur.field("SpecifiedColor"), V::Struct(_));
                let mut c = color_of(&cur, [1.0; 4]);
                for (i, n) in ["RedCurve", "GreenCurve", "BlueCurve", "AlphaCurve"].iter().enumerate() {
                    if let Some(v) = ch(n) {
                        c[i] = v;
                    }
                }
                let v = if slate { V::st(&[("SpecifiedColor", lin(c)), ("ColorUseRule", V::Int(0))]) } else { lin(c) };
                vm.set(w, &t.property, v);
            }
            "Transform" => {
                let cur = vm.prop(w, "RenderTransform");
                let g = |k: &str, d: [f64; 2]| v2_of(cur.field(k), d);
                let (mut tr, mut sc, mut sh) = (g("Translation", [0.0, 0.0]), g("Scale", [1.0, 1.0]), g("Shear", [0.0, 0.0]));
                let mut ang = cur.field("Angle").f();
                for i in 0..2 {
                    let sfx = if i == 0 { String::new() } else { format!("[{i}]") };
                    if let Some(v) = ch(&format!("Translation{sfx}")) {
                        tr[i] = v;
                    }
                    if let Some(v) = ch(&format!("Scale{sfx}")) {
                        sc[i] = v;
                    }
                    if let Some(v) = ch(&format!("Shear{sfx}")) {
                        sh[i] = v;
                    }
                }
                if let Some(v) = ch("Rotation") {
                    ang = v;
                }
                let v2 = |a: [f64; 2]| V::st(&[("X", V::Float(a[0])), ("Y", V::Float(a[1]))]);
                vm.set(w, "RenderTransform", V::st(&[("Translation", v2(tr)), ("Scale", v2(sc)), ("Shear", v2(sh)), ("Angle", V::Float(ang))]));
            }
            "Margin" => {
                let cur = margin_of(&vm.prop(w, &t.property));
                let mut m = cur;
                for (i, n) in ["LeftCurve", "TopCurve", "RightCurve", "BottomCurve"].iter().enumerate() {
                    if let Some(v) = ch(n) {
                        m[i] = v;
                    }
                }
                vm.set(w, &t.property, V::st(&[("Left", V::Float(m[0])), ("Top", V::Float(m[1])), ("Right", V::Float(m[2])), ("Bottom", V::Float(m[3]))]));
            }
            _ => {}
        }
    }
}

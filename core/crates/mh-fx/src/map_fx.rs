//! Map effects: every ParticleSystemComponent placed in a map's levels (Emitter actors and Blueprint actors' particle
//! components: the component export, its properties over its Template archetype via mh_level Pkgs::props) with its
//! world transform and Template; `FxRequest`s for the auto-activated ones (UActorComponent::bAutoActivate; the
//! ParticleSystemComponent default true is UNCONFIRMED: its ctor not read).

use mh_level::level::{levels, Pkgs};
use serde_json::{Map, Value};

#[derive(Clone, Debug)]
pub struct MapEmitter {
    pub level: String,
    pub name: String,
    /// the actor class it belongs to (the Outer's class)
    pub owner: String,
    /// ParticleSystem package
    pub template: String,
    /// world position (UE cm) and the component's X axis (its forward) in world space
    pub pos_ue: [f64; 3],
    pub dir_ue: [f64; 3],
    pub auto_activate: bool,
    pub props: Map<String, Value>,
}

fn ty(e: &Value) -> &str {
    e.get("Type").and_then(Value::as_str).unwrap_or("")
}

pub fn map_emitters(pk: &Pkgs, map: &str) -> Vec<MapEmitter> {
    let mut out = vec![];
    for lv in levels(pk, map) {
        let ex = pk.load_pkg(&lv.pkg);
        for e in ex.iter() {
            if ty(e) != "ParticleSystemComponent" {
                continue;
            }
            let p = pk.props(e);
            let template = mh_assets::material::strip_index(&mh_assets::material::ue_pkg_path(p.get("Template").unwrap_or(&Value::Null))).to_string();
            if template.is_empty() {
                continue;
            }
            let xf = lv.xf * pk.world_xf(e);
            let o = xf.translation();
            let x = xf.apply([1.0, 0.0, 0.0]);
            let owner = e.get("Outer").and_then(|o| o.get("ObjectName")).and_then(Value::as_str).and_then(|o| o.split('\'').next()).unwrap_or("").to_string();
            out.push(MapEmitter {
                level: lv.pkg.clone(),
                name: e.get("Name").and_then(Value::as_str).unwrap_or("").to_string(),
                owner,
                template,
                pos_ue: o,
                dir_ue: [x[0] - o[0], x[1] - o[1], x[2] - o[2]],
                auto_activate: p.get("bAutoActivate").and_then(Value::as_bool).unwrap_or(true),
                props: p,
            });
        }
    }
    out
}

//! sim_core.rs - the R3 sim adapter, prepared in R2: `SimBackend` over mordhau-core's combat `World` (rust-combat;
//! port of godot/game/combat/combat_state.gd, golden-trace parity in core/tests/golden.rs). `--sim core` selects it.
//!
//! Data: the combat spec is the record dump godot/tools/export_golden.gd writes (core/tests/golden/spec.json, read by
//! mordhau_core::data::RecordsJson, git-ignored Triternion data). When mh-spec (the sheets builder's spec) lands it is
//! one more SpecSource. Without the file the runtime falls back to StubSim and says so in the evidence.
//!
//! Scope (what R3 adds): movement is not in the combat core (mh-character), so fighters keep their spawn location;
//! bone transforms for traces (Fighter.bone_xf) are not fed yet, so attacks run their motion timeline but never hit.

use crate::sim::{FighterView, FrameInput, SimBackend};
use std::collections::HashMap;
use mordhau_core::combat::{Input, MotionKind, World};
use mordhau_core::data::{RecordsJson, SpecSource};
use std::rc::Rc;

/// One-handed arming sword: the weapon the R1 idle clip (1H_RH_Idle ... SwordNew) belongs to; path as in
/// core/tests/scenarios/combat.json "weapons.ARMING".
pub const DEFAULT_WEAPON: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/OneHanded/BP_ArmingSword";

pub struct CoreSim {
    pub world: World,
    locs: Vec<([f32; 3], f32)>,
    names: Vec<String>,
}

impl CoreSim {
    /// `dt` = the runtime's fixed step (sim.rs TICK_HZ). The golden scenarios run at 0.001 s; the rules are written per
    /// tick against world time, so any step works, but parity is only checked at the scenarios' dt.
    /// From a built Spec (mh-host: the spec matrix + paks, the default)
    pub fn from_spec(spec: mordhau_core::data::Spec, dt: f64) -> Result<CoreSim, String> {
        if spec.weapon(DEFAULT_WEAPON).is_none() {
            return Err(format!("spec has no {DEFAULT_WEAPON}"));
        }
        Ok(CoreSim { world: World::new(Rc::new(spec), dt), locs: Vec::new(), names: Vec::new() })
    }

    /// From the reference's golden record dump (`--golden`: core/tests/golden/spec.json)
    pub fn load(spec_path: &std::path::Path, dt: f64) -> Result<CoreSim, String> {
        let txt = std::fs::read_to_string(spec_path).map_err(|e| format!("{}: {e}", spec_path.display()))?;
        let spec = RecordsJson(&txt).load_spec()?;
        if spec.weapon(DEFAULT_WEAPON).is_none() {
            return Err(format!("spec has no {DEFAULT_WEAPON}"));
        }
        Ok(CoreSim { world: World::new(Rc::new(spec), dt), locs: Vec::new(), names: Vec::new() })
    }
}

pub fn state_name(w: &World, i: usize) -> String {
    let f = &w.fighters[i];
    let Some(id) = f.motion else { return "Idle".into() };
    let Some(Some(m)) = f.motions.get(id.0 as usize) else { return "Idle".into() };
    let kind = match &m.k {
        MotionKind::Idle => "Idle",
        MotionKind::Attack(_) => "Attack",
        MotionKind::Parry(_) => "Parry",
        MotionKind::Feinted(_) => "Feinted",
        MotionKind::Blocked(_) => "Blocked",
        MotionKind::Flinch(_) => "Flinch",
        MotionKind::Stun(_) => "Stun",
        MotionKind::Disarmed(_) => "Disarmed",
        MotionKind::Climbing(_) => "Climbing",
        // rust-combat r4: UEnterVehicleMotion / ULeaveVehicleMotion (mordhau-core combat/horse.rs)
        MotionKind::Vehicle(_) => "Vehicle",
        // rust-combat r5: the ranged motions (mordhau-core combat/rangedmotion.rs)
        MotionKind::Ranged(_) => "Ranged",
        // fp-anim: UEquipmentModeSwitchMotion (mordhau-core; compile fix by first-person r3)
        MotionKind::ModeSwitch(_) => "ModeSwitch",
    };
    let bp = m.bp.rsplit('/').next().unwrap_or("");
    if bp.is_empty() { kind.into() } else { format!("{kind}:{bp}") }
}

impl SimBackend for CoreSim {
    fn name(&self) -> &'static str {
        "mordhau-core"
    }
    fn spawn(&mut self, loc: [f32; 3], yaw: f32, team: Option<i64>) -> u32 {
        let id = self.names.len() as u32;
        let name = format!("F{id}");
        let fi = self.world.add_fighter(&name, DEFAULT_WEAPON, "");
        if let Some(t) = team {
            self.world.fighters[fi].team = t;
        }
        self.names.push(name);
        self.locs.push((loc, yaw));
        id
    }
    fn tick(&mut self, _dt: f32, inputs: &HashMap<u32, FrameInput>) {
        for (id, i) in inputs {
            let Some(who) = self.names.get(*id as usize).cloned() else { continue };
            if i.release_block {
                self.world.input(Input::ReleaseBlock { who: who.clone() });
            }
            if let Some(bt) = i.parry {
                self.world.input(Input::Parry { who: who.clone(), bt });
            }
            if i.feint {
                self.world.input(Input::Feint { who: who.clone() });
            }
            if let Some((mv, angle)) = i.attack {
                self.world.input(Input::Attack { who: who.clone(), mv, angle });
            }
            if i.toggle_mode {
                self.world.input(Input::ToggleMode { who });
            }
        }
        self.world.step();
    }
    fn ticks(&self) -> u64 {
        self.world.tick_n.max(0) as u64
    }
    fn fighters(&self) -> Vec<FighterView> {
        self.names
            .iter()
            .enumerate()
            .filter_map(|(i, n)| {
                let fi = self.world.fighter_index(n)?;
                let f = &self.world.fighters[fi];
                Some(FighterView {
                    id: i as u32,
                    team: Some(f.team),
                    loc: self.locs[i].0,
                    half_height: 96.0, // AMordhauCharacter ctor CapsuleHalfHeight 96 (mh-character exe.rs)
                    vel: [0.0; 3],
                    falling: false,
                    crouched: false,
                    look_up: f.look_up_value as f32,
                    yaw: self.locs[i].1,
                    state: state_name(&self.world, fi),
                    health: f.health as f32,
                    stamina: f.stamina,
                })
            })
            .collect()
    }
    fn combat(&self) -> Option<&World> {
        Some(&self.world)
    }
    fn fighter_index(&self, id: u32) -> Option<usize> {
        self.world.fighter_index(self.names.get(id as usize)?)
    }
    fn debug(&self) -> serde_json::Value {
        let n = self.world.events.len();
        serde_json::json!({"now": self.world.now, "dt": self.world.dt, "events_total": n,
            "events_tail": self.world.events[n.saturating_sub(20)..].to_vec(),
            "fighters": self.world.fighters.iter().map(|f| serde_json::json!({"name": f.name, "stamina": f.stamina,
                "health": f.health, "weapon": f.weapon_path})).collect::<Vec<_>>()})
    }
}

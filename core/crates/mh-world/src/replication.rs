//! Client replication of the world actors for mh-net. It provides every replicated property's authority value
//! (`rep_vars`) and the client side of its RepNotify (`apply_rep`), with the OnRep's authority-only branches left out.
//! A client World is built the same way, then `set_authority(false)`. That leaves the server logic out of `tick`
//! (spawners, capture-point timers, door / ladder reactions, destructible regeneration, pushable presence) and runs
//! the pushables' network interpolation instead.
//!
//! Properties (names as in the classes, values as i64):
//! - BP_Door DoorState (byte; OnRep_DoorState@25..@890 = `Door::set_state`);
//! - BP_DestroyableActor ReplicatedHealth (byte; OnRep_ReplicatedHealth) and Regenerating (bool; OnRep_Regenerating
//!   plays only the regeneration sound). Both also cover BP_FrontlineDestroyable and the destroyable doors;
//! - BP_Ladder LadderState (byte; OnRep_LadderState -> BeginAnimatingLadder);
//! - BP_SwitchInteractable Value (bool; OnRep_Value -> OnValueToggled / PreventInteraction);
//! - APushableActor ReplicatedProgress (uint16; OnRep_ReplicatedProgress rva 0x1666670), sent only with
//!   bReplicateProgress (PreReplication rva 0x166b0e0).
//!
//! AControlPoint's ReplicatedCaptureProgress / OwningTeam / CapturingTeam belong to mh-mode's control point, which
//! already has on_rep_replicated_capture_progress. A client World learns the owner through `set_capture_point_owner`.

use crate::destructible::{Destructible, Overlaps};
use crate::world::{ActorId, Kind, Queries, World, WorldEvent};

fn replicates_progress(p: &crate::props::Props) -> bool {
    // bReplicateProgress: ctor true (APushableActor ctor rva 0x1642ff0)
    p.get("bReplicateProgress").and_then(|v| v.as_bool()).unwrap_or(true)
}

impl World {
    /// Every replicated variable's current value: (actor, property name, value)
    pub fn rep_vars(&self) -> Vec<(ActorId, &'static str, i64)> {
        let mut out = vec![];
        for a in &self.actors {
            let health = |out: &mut Vec<(ActorId, &'static str, i64)>, h: &Destructible| {
                out.push((a.id, "ReplicatedHealth", h.replicated as i64));
                out.push((a.id, "Regenerating", h.regenerating as i64));
            };
            match &a.kind {
                Kind::Door(d) => {
                    out.push((a.id, "DoorState", d.state as i64));
                    health(&mut out, &d.health);
                }
                Kind::Destructible(x) => health(&mut out, x),
                Kind::Objective(o) => {
                    if o.kind == crate::frontline::ObjectiveKind::Destroyable {
                        health(&mut out, &o.base);
                    }
                    if let Some(p) = &o.push {
                        if replicates_progress(&a.props) {
                            out.push((a.id, "ReplicatedProgress", p.replicated_progress as i64));
                        }
                    }
                }
                Kind::Pushable(p) => {
                    if replicates_progress(&a.props) {
                        out.push((a.id, "ReplicatedProgress", p.replicated_progress as i64));
                    }
                }
                Kind::Ladder(l) => out.push((a.id, "LadderState", l.state as i64)),
                Kind::ProgressDriver(d) => out.push((a.id, "Value", d.value as i64)),
                _ => {}
            }
        }
        out
    }

    /// The client receiving `var = value` for `actor`: set it and run its OnRep as a non-authority machine. Unknown
    /// actor / property pairs are ignored
    pub fn apply_rep(&mut self, actor: ActorId, var: &str, value: i64, _q: &dyn Queries) -> Vec<WorldEvent> {
        let mut ev = vec![];
        let now = self.now;
        let Some(a) = self.actors.get_mut(actor) else { return ev };
        let props = a.props.clone();
        let none = Overlaps::default;
        let on_health = |h: &mut Destructible, ev: &mut Vec<WorldEvent>| {
            let v = value.clamp(0, 255) as u8;
            if h.replicated != v {
                h.replicated = v;
                h.on_rep_replicated_health(actor, &props, now, &none, ev);
            }
        };
        match (&mut a.kind, var) {
            (Kind::Door(d), "DoorState") => {
                let s = value.clamp(0, 255) as u8;
                if s != d.state {
                    d.set_state(s, now);
                }
            }
            (Kind::Door(d), "ReplicatedHealth") => on_health(&mut d.health, &mut ev),
            (Kind::Destructible(x), "ReplicatedHealth") => on_health(x, &mut ev),
            (Kind::Objective(o), "ReplicatedHealth") => {
                let before = o.base.replicated;
                on_health(&mut o.base, &mut ev);
                // OnReplicatedHealthChanged -> CapturePoint.ObjectivesChanged (BP_FrontlineDestroyable@1513) on clients
                if o.base.replicated != before {
                    if let Some(cp) = o.capture_point {
                        ev.push(WorldEvent::ObjectivesChanged { capture_point: cp });
                    }
                }
            }
            (Kind::Door(d), "Regenerating") => d.health.regenerating = value != 0,
            (Kind::Destructible(x), "Regenerating") => x.regenerating = value != 0,
            (Kind::Objective(o), "Regenerating") => o.base.regenerating = value != 0,
            (Kind::Ladder(l), "LadderState") => l.on_rep_state(value.clamp(0, 255) as u8, now),
            (Kind::ProgressDriver(d), "Value") => d.on_rep_value(&props, value != 0, now),
            (Kind::Pushable(p), "ReplicatedProgress") => {
                p.on_rep_progress(actor, value.clamp(0, 0xffff) as u16, now, &mut ev);
            }
            (Kind::Objective(o), "ReplicatedProgress") => {
                if let Some(p) = o.push.as_deref_mut() {
                    if p.on_rep_progress(actor, value.clamp(0, 0xffff) as u16, now, &mut ev) {
                        o.progress = p.progress;
                        if let Some(cp) = o.capture_point {
                            ev.push(WorldEvent::ObjectivesChanged { capture_point: cp });
                        }
                    }
                }
            }
            _ => {}
        }
        self.settle_client(&mut ev);
        ev
    }

    /// A client's capture point recomputes ObjectiveProgress and tells its objectives (ObjectivesChanged before the
    /// HasAuthority branch @1615); the authority part is skipped (`CapturePoint::authority_changed` is server-only)
    pub(crate) fn settle_client(&mut self, ev: &mut Vec<WorldEvent>) {
        let cps: Vec<ActorId> = ev.iter().filter_map(|e| if let WorldEvent::ObjectivesChanged { capture_point } = e { Some(*capture_point) } else { None }).collect();
        for cp in cps {
            self.objectives_changed(cp, ev);
        }
    }

    /// Make this World a client (false) or the server (true): the per-actor HasAuthority flags follow
    pub fn set_authority(&mut self, authority: bool) {
        self.authority = authority;
        for a in self.actors.iter_mut() {
            match &mut a.kind {
                Kind::Door(d) => {
                    d.authority = authority;
                    d.health.authority = authority;
                }
                Kind::Destructible(x) => x.authority = authority,
                Kind::Objective(o) => o.base.authority = authority,
                Kind::Ladder(l) => l.authority = authority,
                _ => {}
            }
        }
    }
}

//! Replication of the placed gameplay actors' state (mh-world: doors, destructibles, ladders, switches / progress
//! drivers, capture points, pushables) from the server to the clients. The replicated variables are the exe's and the
//! packages' (read in rust-net r6):
//!   - BP_Door DoorState (byte, RepNotify OnRep_DoorState); BP_DestroyableActor ReplicatedHealth (byte,
//!     OnRep_ReplicatedHealth) and Regenerating (bool, OnRep_Regenerating) - also BP_FrontlineDestroyable and the
//!     destroyable doors; BP_Ladder LadderState (byte, OnRep_LadderState); BP_SwitchInteractable Value (bool,
//!     OnRep_Value): the Net-flagged properties of their BlueprintGeneratedClass exports (extract/json packages
//!     Mordhau/Content/Mordhau/Blueprints/Interactables/...)
//!   - AControlPoint ReplicatedCaptureProgress, OwningTeam, CapturingTeam (AControlPoint::GetLifetimeReplicatedProps
//!     rva=0x14fb7c0; OnRep_ReplicatedCaptureProgress rva=0x150ecf0 takes Byte * 0.003921569 on the first update)
//!   - APushableActor ReplicatedProgress (u16 = trunc(Progress * 65535): SetProgress rva=0x166d4c0; GetLifetimeReplicatedProps
//!     rva=0x165cb40, active while bReplicateProgress: PreReplication rva=0x166b0e0; OnRep rva=0x1666670)
//! When they go out: the Blueprint actors call ForceNetUpdate / FlushNetDormancy where they change them
//! (state/world_kismet BP_Door, BP_DestroyableActor, BP_Ladder, BP_SwitchInteractable bytecode), so a change goes out
//! in the next server frame although BP_DestroyableActor's CDO NetUpdateFrequency is 0.5. The natives are polled at
//! their NetUpdateFrequency: AControlPoint 10 (ctor rva=0x14e30e0, 0x1414e31d6), APushableActor 10 (ctor rva=0x1642ff0).
//! Properties are reliable (re-sent until acknowledged in the engine), so the reliable channel carries them.
//! UNCONFIRMED: relevancy of world actors (AControlPoint is bAlwaysRelevant in BP_CapturePoint; the others use the
//! default 15000 cm cull distance, not applied here - every client gets every actor); dormancy itself.
//!
//! The world side (reading the authority values, running the OnReps on a client) is the host's world - mh-world's
//! `World` through `RepWorld`.

use std::collections::BTreeMap;

use crate::msg::Msg;

/// What a world gives the replication: every replicated variable's authority value, and the client side's OnRep
pub trait RepWorld {
    /// (actor id, property name, value as an integer: bytes / bools / the pushable's u16)
    fn rep_vars(&self) -> Vec<(u32, String, i64)>;
    /// a client received a new value: set it and run the property's OnRep as a non-authority machine does
    fn apply_rep(&mut self, actor: u32, var: &str, value: i64);
}

/// seconds between two updates of an actor that only replicates at its NetUpdateFrequency (0: every frame it changed)
pub fn update_period(var: &str) -> f64 {
    match var {
        // AControlPoint ctor 0x1414e31d6: NetUpdateFrequency 10
        "ReplicatedCaptureProgress" | "OwningTeam" | "CapturingTeam" => 0.1,
        // APushableActor ctor rva=0x1642ff0: NetUpdateFrequency 10
        "ReplicatedProgress" => 0.1,
        // the Blueprint actors ForceNetUpdate / FlushNetDormancy on change
        _ => 0.0,
    }
}

/// The server's per-connection shadow of what each client has
#[derive(Default)]
pub struct WorldRepServer {
    sent: BTreeMap<u32, BTreeMap<(u32, String), i64>>,
    last: BTreeMap<(u32, u32), f64>,
    pub props_sent: u64,
}

impl WorldRepServer {
    /// the changed variables due for `client` at server time `now`, as WorldProp messages (the shadow is updated)
    pub fn diff(&mut self, client: u32, now: f64, vars: &[(u32, String, i64)]) -> Vec<Msg> {
        let shadow = self.sent.entry(client).or_default();
        let mut out = vec![];
        for (actor, var, v) in vars {
            if shadow.get(&(*actor, var.clone())) == Some(v) {
                continue;
            }
            let period = update_period(var);
            let key = (client, *actor);
            if period > 0.0 && self.last.get(&key).is_some_and(|t| now - t < period) && shadow.contains_key(&(*actor, var.clone())) {
                continue; // not yet due at this actor's NetUpdateFrequency (the initial replication is never held)
            }
            shadow.insert((*actor, var.clone()), *v);
            out.push(Msg::WorldProp { actor: *actor, var: var.clone(), value: *v });
        }
        // an actor that sent this frame waits a full period for its next update
        for m in &out {
            if let Msg::WorldProp { actor, var, .. } = m {
                if update_period(var) > 0.0 {
                    self.last.insert((client, *actor), now);
                }
            }
        }
        self.props_sent += out.len() as u64;
        out
    }

    /// a client left: forget what it had
    pub fn forget(&mut self, client: u32) {
        self.sent.remove(&client);
        self.last.retain(|k, _| k.0 != client);
    }
}

/// RepWorld over mh-world's World (its replication API, mh-world r2 replication.rs): the server reads `rep_vars`; a
/// client runs `apply_rep` with its Queries and collects the WorldEvents the OnReps return for the host to apply.
/// Clients call `World::set_authority(false)` first. AControlPoint's variables stay mh-mode's (not in rep_vars).
#[cfg(feature = "world")]
pub struct MhWorld<'a> {
    pub w: &'a mut mh_world::World,
    pub q: &'a dyn mh_world::Queries,
    pub events: Vec<mh_world::WorldEvent>,
}

#[cfg(feature = "world")]
impl<'a> MhWorld<'a> {
    pub fn new(w: &'a mut mh_world::World, q: &'a dyn mh_world::Queries) -> Self {
        MhWorld { w, q, events: vec![] }
    }
}

#[cfg(feature = "world")]
impl RepWorld for MhWorld<'_> {
    fn rep_vars(&self) -> Vec<(u32, String, i64)> {
        self.w.rep_vars().into_iter().map(|(a, v, x)| (a as u32, v.to_string(), x)).collect()
    }
    fn apply_rep(&mut self, actor: u32, var: &str, value: i64) {
        let ev = self.w.apply_rep(actor as mh_world::ActorId, var, value, self.q);
        self.events.extend(ev);
    }
}

//! BP_Door (Mordhau/Content/Mordhau/Blueprints/Interactables/Environment/BP_Door; parent BP_DestroyableActor): the
//! 6-state door, its leaf rotation, interaction, kick open, and the leaf pushing / bouncing off characters.
//! Bytecode: state/world_kismet/BP_Door.txt.
//!
//! DoorState (E_DoorState byte, OnRep_DoorState@25..@890): 0 closed, 1 open forward, 2 fast open forward (kicked),
//! 3 open backward, 4 fast open backward, 5 fast close. OnRep sets the leaf's target yaw and speed:
//! 0 -> DoorYawClosed / DoorCloseSpeed, 1 -> DoorYawOpenForward / DoorOpenSpeed, 2 -> DoorYawOpenForward /
//! DoorOpenSpeedFast (+ FastOpenStart), 3 -> DoorYawOpenBackward / DoorOpenSpeed, 4 -> DoorYawOpenBackward /
//! DoorOpenSpeedFast, 5 -> DoorYawClosed / DoorOpenSpeedFast.

use crate::destructible::{Destructible, Ov};
use crate::props::{finterp_to_constant, nearly_equal, Props};
use crate::world::{ActorId, CharId, CharView, DamageInfo, Queries, WorldEvent};
use crate::V;
use mh_level::xf::Xf;

#[derive(Clone, Debug)]
pub struct Door {
    pub state: u8,
    pub last_completed: u8,
    /// the leaf's current / target relative yaw (degrees) and speed (deg/s)
    pub yaw: f64,
    pub target: f64,
    pub speed: f64,
    pub tick_enabled: bool,
    pub has_begun_play: bool,
    pub fast_open_start: f64,
    pub kicker: Option<CharId>,
    pub will_ragdoll: bool,
    pub yaw_closed: f64,
    pub yaw_fwd: f64,
    pub yaw_back: f64,
    pub open_speed: f64,
    pub close_speed: f64,
    pub fast_speed: f64,
    pub can_fwd: bool,
    pub can_back: bool,
    pub can_kick: bool,
    /// DamageParticleTransform's relative location (component template), the push origin
    pub push_origin_rel: V,
    /// the door is also a destructible (BP_DestroyableActor parent; DamageFactor 0 on BP_Door = invulnerable)
    pub health: Destructible,
    pub props: Props,
    /// HasAuthority (a client World leaves the @3488 / @5257 branches to the server)
    pub authority: bool,
}

/// a component template's RelativeLocation, looked up the Blueprint chain (<Name>_GEN_VARIABLE exports)
pub fn component_rel_location(pk: &mh_level::Pkgs, chain: &[String], comp: &str) -> Option<V> {
    for cls in chain {
        for e in pk.load_pkg(cls).iter() {
            if e.get("Name").and_then(|v| v.as_str()) == Some(&format!("{comp}_GEN_VARIABLE")) {
                let r = e.get("Properties").and_then(|p| p.get("RelativeLocation"));
                let g = |k: &str| r.and_then(|r| r.get(k)).and_then(|v| v.as_f64()).unwrap_or(0.0);
                return Some([g("X"), g("Y"), g("Z")]);
            }
        }
    }
    None
}

fn sub(a: V, b: V) -> V {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

impl Door {
    pub fn new(p: &Props, pk: &mh_level::Pkgs, chain: &[String], _xf: &Xf) -> Door {
        Door {
            state: 0,
            last_completed: 0,
            yaw: 0.0,
            target: 0.0,
            speed: 0.0,
            tick_enabled: false,
            has_begun_play: false,
            fast_open_start: 0.0,
            kicker: None,
            will_ragdoll: false,
            yaw_closed: p.f("DoorYawClosed"),
            yaw_fwd: p.f("DoorYawOpenForward"),
            yaw_back: p.f("DoorYawOpenBackward"),
            open_speed: p.f("DoorOpenSpeed"),
            close_speed: p.f("DoorCloseSpeed"),
            fast_speed: p.f("DoorOpenSpeedFast"),
            can_fwd: p.b("CanOpenForwards"),
            can_back: p.b("CanOpenBackwards"),
            can_kick: p.b("CanBeKickedOpen"),
            push_origin_rel: component_rel_location(pk, chain, "DamageParticleTransform").unwrap_or([0.0; 3]),
            health: Destructible::new(p),
            props: p.clone(),
            authority: true,
        }
    }

    /// UserConstructionScript + ReceiveBeginPlay: StartingState 0 / 1 / 2 -> DoorState 0 / 1 / 3
    /// (ConvertStartingStateToDoorState) with the leaf already at that yaw (UserConstructionScript@329 / @496 / @663)
    pub fn begin_play(&mut self, id: ActorId, ev: &mut Vec<WorldEvent>) {
        let start = self.props.i("StartingState");
        let (state, yaw) = match start {
            1 => (1, self.yaw_fwd),
            2 => (3, self.yaw_back),
            _ => (0, self.yaw_closed),
        };
        self.yaw = yaw;
        self.set_state(state, 0.0);
        self.target = yaw;
        self.yaw = yaw;
        self.last_completed = state;
        self.has_begun_play = true;
        ev.push(WorldEvent::ComponentYaw { actor: id, component: "StaticMesh", yaw_deg: self.yaw });
    }

    /// DoorState = s; OnRep_DoorState
    pub fn set_state(&mut self, s: u8, now: f64) {
        self.state = s;
        self.tick_enabled = true;
        match s {
            0 => {
                self.target = self.yaw_closed;
                self.speed = self.close_speed;
            }
            1 => {
                self.target = self.yaw_fwd;
                self.speed = self.open_speed;
            }
            2 => {
                self.target = self.yaw_fwd;
                self.speed = self.fast_speed;
                self.fast_open_start = now;
            }
            3 => {
                self.target = self.yaw_back;
                self.speed = self.open_speed;
            }
            4 => {
                self.target = self.yaw_back;
                self.speed = self.fast_speed;
                self.fast_open_start = now;
            }
            _ => {
                self.target = self.yaw_closed;
                self.speed = self.fast_speed;
                self.fast_open_start = now;
            }
        }
    }

    /// GetDoorForwardVector: ComposeRotators(ActorRotation, NormalizedDeltaRotator(leaf yaw, DoorYawClosed)).Vector():
    /// the actor's forward turned about world Z by the leaf's yaw from closed
    pub fn door_forward(&self, xf: &Xf) -> V {
        let fx = [xf.m[0][0], xf.m[1][0], xf.m[2][0]];
        let l = (fx[0] * fx[0] + fx[1] * fx[1] + fx[2] * fx[2]).sqrt().max(1e-12);
        let f = fx.map(|c| c / l);
        let mut d = (self.yaw - self.yaw_closed) % 360.0;
        if d > 180.0 {
            d -= 360.0;
        } else if d <= -180.0 {
            d += 360.0;
        }
        let (s, c) = d.to_radians().sin_cos();
        [f[0] * c - f[1] * s, f[0] * s + f[1] * c, f[2]]
    }

    /// GetDoorToActorAngle: DegAcos(Dot(door forward, actor forward))
    pub fn angle_to(&self, xf: &Xf, ch: &CharView) -> f64 {
        let f = self.door_forward(xf);
        let d = (f[0] * ch.forward[0] + f[1] * ch.forward[1] + f[2] * ch.forward[2]).clamp(-1.0, 1.0);
        d.acos().to_degrees()
    }

    /// OnInteractionStart (ExecuteUbergraph@5840): an open / opening door closes; a closed (or fast-closing) door
    /// opens forward when (angle >= 90 or it cannot open backwards) and it can open forwards, else backwards
    pub fn interact(&mut self, xf: &Xf, ch: &CharView, now: f64, _ev: &mut Vec<WorldEvent>) {
        let angle = self.angle_to(xf, ch);
        if self.state != 0 && self.state != 5 {
            self.set_state(0, now); // @1052
            return;
        }
        if (angle >= 90.0 || !self.can_back) && self.can_fwd {
            if self.state != 1 && self.state != 2 {
                self.set_state(1, now); // @1136
            }
        } else if self.state != 3 && self.state != 4 && self.can_back {
            self.set_state(3, now); // @1220
        }
    }

    /// HandleFastOpen(Char, OnlyClosed, WantsRagdoll) -> kicked open
    pub fn handle_fast_open(&mut self, xf: &Xf, ch: &CharView, only_closed: bool, wants_ragdoll: bool, now: f64) -> bool {
        if only_closed && self.state != 0 && self.state != 5 {
            return false; // @138
        }
        let angle = self.angle_to(xf, ch);
        let closed = self.state == 0 || self.state == 5;
        let new = if angle >= 90.0 {
            if angle < 120.0 || self.state == 1 || self.state == 2 {
                return false; // @1111
            }
            if !closed {
                5 // @1127
            } else if self.can_fwd {
                2 // @548
            } else {
                return false;
            }
        } else if angle <= 60.0 {
            if self.state == 3 || self.state == 4 {
                return false;
            }
            if !closed {
                5 // @1043
            } else if self.can_back {
                4 // @965
            } else {
                return false;
            }
        } else {
            return false;
        };
        self.set_state(new, now);
        self.kicker = Some(ch.id); // @611
        self.will_ragdoll = wants_ragdoll;
        true
    }

    /// ReceiveAnyDamage (ExecuteUbergraph@6412): a kick (attack Move 4 by a character) fast-opens a closed door when
    /// CanBeKickedOpen; otherwise the BP_DestroyableActor damage path
    #[allow(clippy::too_many_arguments)]
    pub fn receive_any_damage(&mut self, id: ActorId, xf: &Xf, amount: f64, d: &DamageInfo, now: f64, ov: Ov, ev: &mut Vec<WorldEvent>) {
        if self.can_kick && d.attack_move == Some(4) {
            if let (Some(c), Some(f)) = (d.causer, d.causer_forward) {
                let v = CharView { id: c, forward: f, ..Default::default() };
                if self.handle_fast_open(xf, &v, true, false, now) {
                    return;
                }
            }
        }
        let p = self.props.clone();
        self.health.receive_any_damage(id, &p, amount, d, now, ov, ev);
    }

    /// ReceiveTick (@4907): settled -> LastCompletedDoorState = DoorState, tick off, depenetrate overlapping
    /// characters (@3488..@4336: K2_TeleportTo own location); moving -> FInterpTo_Constant the leaf, then (authority)
    /// damage non-door MordhauActors it overlaps by 1000 (@5292..@3416, first one) and react to characters it
    /// overlaps (@5457..): a fast-moving leaf (states 2, 4, 5) ragdolls + knocks back (1100 cm/s) the ones attacking /
    /// blocked / feinting when WillRagdoll, else bounces closed; a slow-opening leaf bounces closed (@1304); a closing
    /// leaf pushes them away (4000 * dt, @1339..@2139)
    #[allow(clippy::too_many_arguments)]
    pub fn tick(&mut self, id: ActorId, xf: &Xf, dt: f64, now: f64, q: &dyn Queries, chars: &[CharView], ov: Ov, ev: &mut Vec<WorldEvent>) {
        self.health.tick(id, &self.props.clone(), now, ov, ev);
        if !self.tick_enabled {
            return;
        }
        if nearly_equal(self.yaw, self.target, 1e-6) {
            self.last_completed = self.state;
            self.tick_enabled = false;
            if !self.authority {
                return; // @3508
            }
            for c in q.overlapping_chars(id) {
                if chars.iter().any(|v| v.id == c && !v.in_vehicle) {
                    ev.push(WorldEvent::Depenetrate { char: c });
                }
            }
            return;
        }
        self.yaw = finterp_to_constant(self.yaw, self.target, dt, self.speed);
        ev.push(WorldEvent::ComponentYaw { actor: id, component: "StaticMesh", yaw_deg: self.yaw });
        if !self.authority {
            return; // @5277
        }
        if let Some(a) = q.overlapping_actors(id).first() {
            ev.push(WorldEvent::DamageActor { actor: *a, amount: 1000.0 }); // non-door actors only [UNCONFIRMED: target decoded as null]
        }
        let overlapping: Vec<&CharView> = q.overlapping_chars(id).iter().filter_map(|c| chars.iter().find(|v| v.id == *c)).collect();
        if overlapping.is_empty() {
            return;
        }
        let origin = xf.apply(self.push_origin_rel);
        let dir2 = |v: &CharView| {
            let d = sub(v.location, origin);
            let l = (d[0] * d[0] + d[1] * d[1]).sqrt();
            if l > 1e-8 { [d[0] / l, d[1] / l, 0.0] } else { [0.0; 3] }
        };
        if matches!(self.state, 2 | 4 | 5) {
            for v in overlapping {
                if Some(v.id) == self.kicker || v.in_vehicle {
                    continue;
                }
                if self.will_ragdoll && !v.in_knockback && !v.ragdoll_falling && v.attacking_or_blocked_or_feinted {
                    ev.push(WorldEvent::Ragdoll { char: v.id });
                    ev.push(WorldEvent::Knockback { char: v.id, impulse: dir2(v).map(|c| c * 1100.0) });
                } else if (!self.will_ragdoll || !v.in_knockback) && self.state != 0 {
                    self.set_state(0, now); // @86..@186
                }
            }
        } else if self.state != 0 {
            self.set_state(0, now); // @1304
        } else {
            for v in overlapping {
                if !v.in_vehicle {
                    ev.push(WorldEvent::Knockback { char: v.id, impulse: dir2(v).map(|c| c * 4000.0 * dt) });
                }
            }
        }
    }
}

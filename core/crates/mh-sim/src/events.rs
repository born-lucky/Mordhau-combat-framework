//! Sound / presentation events of the Sim (rust-combat r6, for mh-audio game.rs and mh-runtime). Emitted into the same
//! stream as the combat events (`Sim::drain`), in UE units (cm, degrees). Kinds:
//!
//! - "motion" (mordhau-core, already): a motion's OnBegin (kind Attack / Parry / Feinted / Flinch / ...): armour foley.
//! - "release": UAttackMotion::OnTick_Implementation rva=0x16328c0, the attack's first release tick (the flag at
//!   +0x1071) -> AMordhauWeapon::StartWoosh rva=0x1640170 + ReleaseFoley. Fields: who, trace_start / trace_end (the
//!   weapon's CurrentTraceStart / End after this tick's PrepareForTracing), stab (Move Stab / AltStab), release_duration
//!   (ReleaseEnd - WindupEnd; its identity with +0xe54 UNCONFIRMED as in mh-audio).
//! - "crouch_start" / "crouch_end": OnStartCrouch / OnEndCrouch (the movement's bIsCrouched changing).
//! - "fall_damage": UAdvancedCharacterMovement's fall damage (ExeMovement::check_fall_damage queues it) applied
//!   through TakeDamage with EMordhauDamageType Fall (3); fields who, damage, applied.
//! - "foot_landed": the LeftFootLanded / RightFootLanded anim notifies of the lower body (lower.rs) -> DoFootstep ->
//!   AAdvancedCharacter::PlayFootstepSound rva=0x1498960. Fields: who, foot (0 left, 1 right), foot_pos (the foot bone,
//!   world), speed (2D, cm/s), crouched, floor_component (the floor hit's component id, or null), surface (the
//!   floor hit's EPhysicalSurface from `Sim::surface_of`, e.g. mh-level CollisionWorld::surface_at; 0 without one).
//! - "hit" / "died" (mordhau-core, already): damage taken (victim, applied) and death: the voice triggers.

use crate::sim::Sim;
use mordhau_core::combat::system::DAMAGE_FALL;
use serde_json::json;

/// per-fighter edge state for the events above
#[derive(Clone, Debug, Default)]
pub struct EventState {
    /// (motion start time, released) of the current attack
    pub attack: Option<(f64, bool)>,
    /// the attack (start time) whose yell played (bPlayedYell)
    pub yelled: Option<f64>,
    pub crouched: bool,
    /// UAttackMotion TrailWeight (the value AMordhauWeapon::UpdateTrail is fed), see `Sim::trail_weight`
    pub trail: f32,
}

/// UAttackMotion PlayAttackYellTimeReleaseOffset: the motion Blueprint's value over the ctor's (UAttackMotion ctor
/// -0.05, decomp UAttackMotion.cpp 3538; UKickMotion ctor 0.0, UKickMotion.cpp 265)
fn yell_offset(s: &Sim, bp: &str, native: &str) -> f64 {
    let ctor = if native == "UKickMotion" { 0.0 } else { -0.05 };
    match &s.anim {
        Some(a) if !bp.is_empty() => a.rd.defaults(bp).get("PlayAttackYellTimeReleaseOffset").and_then(|v| v.as_f64()).unwrap_or(ctor),
        _ => ctor,
    }
}

impl Sim {
    /// The current attack's TrailWeight (UAttackMotion::OnTick_Implementation rva=0x16328c0, decomp 2879-2890), the
    /// weight AMordhauWeapon::UpdateTrail gets (UAttackMotion::OnLateTick_Implementation rva=0x1631f00); 0 when idle
    pub fn trail_weight(&self, fi: usize) -> f32 {
        self.ev_state.get(fi).map(|e| e.trail).unwrap_or(0.0)
    }

    /// after the movement frame: crouch edges and fall damage
    pub(crate) fn movement_events(&mut self, fi: usize) {
        while self.ev_state.len() <= fi {
            self.ev_state.push(EventState::default());
        }
        let name = self.combat.fighters[fi].name.clone();
        let c = self.movers[fi].is_crouched;
        if c != self.ev_state[fi].crouched {
            self.ev_state[fi].crouched = c;
            self.combat.emit_event(json!({"kind": if c { "crouch_start" } else { "crouch_end" }, "who": name}));
        }
        // rust-character r9: AMordhauCharacter::OnJumped_Implementation rva=0x1554610 (decomp: OffsetStamina(-(int)
        // JumpStaminaCost), then the stamina component's StopRegeneration(0) unless bIgnoresStopping) per jump the
        // movement made this frame (ExeMovement::jumps, OnJumped)
        let jumps = std::mem::take(&mut self.movers[fi].jumps);
        // bJumped for the anim instance (air.rs, fidelity-audit r6)
        if jumps > 0 {
            if let Some(f) = self.fanim.get_mut(fi) {
                f.jumped = true;
            }
        }
        for _ in 0..jumps {
            if self.combat.fighters[fi].dead {
                break;
            }
            let cost = self.movers[fi].c.jump_stamina_cost;
            self.combat.offset_stamina(fi, -(cost as i64));
            self.combat.stop_stamina_regen(fi, 0.0);
            self.combat.emit_event(json!({"kind": "jump", "who": name, "stamina_cost": cost}));
        }
        let falls = std::mem::take(&mut self.movers[fi].fall_damage);
        for d in falls {
            if self.combat.fighters[fi].dead {
                break;
            }
            let applied = self.combat.take_damage(fi, d as f64, None, DAMAGE_FALL);
            self.combat.emit_event(json!({"kind": "fall_damage", "who": name, "damage": d, "applied": applied}));
        }
    }

    /// after the late tick: the first release tick of each attack; the foot notifies of this frame's poses
    pub(crate) fn post_events(&mut self) {
        for fi in 0..self.combat.fighters.len() {
            while self.ev_state.len() <= fi {
                self.ev_state.push(EventState::default());
            }
            let name = self.combat.fighters[fi].name.clone();
            let cur = self.combat.cur_m(fi).and_then(|m| m.attack().map(|a| (m.start_time, a.stage, a.mv, a.release_end - a.windup_end)));
            // UAttackMotion::OnTick_Implementation rva=0x16328c0 (decomp 2580-2604): in Windup / Release, once per motion
            // (bPlayedYell), max(-LagInduction, 0) + WindupEnd + PlayAttackYellTimeReleaseOffset < now ->
            // UCharacterVoiceComponent::PlayAttackYell
            if let Some(m) = self.combat.cur_m(fi) {
                if let Some(at) = m.attack() {
                    let st = m.start_time;
                    if (at.stage == 0 || at.stage == 1) && self.ev_state[fi].yelled != Some(st) {
                        let off = yell_offset(self, &m.bp, &at.native);
                        if (-at.lag_induction).max(0.0) + at.windup_end + off < self.combat.now {
                            self.ev_state[fi].yelled = Some(st);
                            self.combat.emit_event(json!({"kind": "attack_yell", "who": name}));
                        }
                    }
                }
            }
            // UAttackMotion::OnTick_Implementation rva=0x16328c0 (decomp 2879-2890): TrailWeight = 0 without bUsesTrail
            // (UAttackMotion ctor true, decomp 3572; UKickMotion ctor false, src/.../KickMotion.cpp; no Blueprint sets
            // it), 0 in Windup, 1 in Release (fVar19 = 1.0 from decomp 2580), else FInterpConstantTo(w, 0, dt, 5).
            // Without an attack motion the weapon's trail is updated with 0 (OnLeave_Implementation rva=0x16323e0)
            {
                let dt = self.combat.dt as f32;
                let w = self.ev_state[fi].trail;
                let uses = self.combat.cur_m(fi).and_then(|m| m.attack().map(|a| a.native != "UKickMotion"));
                self.ev_state[fi].trail = match (uses, cur.map(|c| c.1)) {
                    (Some(false), _) | (None, _) => 0.0,
                    (Some(true), Some(0)) => 0.0,
                    (Some(true), Some(1)) => 1.0,
                    _ => {
                        let step = 5.0 * dt;
                        if w * w < 1e-8 { 0.0 } else { (w - step.min(w)).max(0.0) }
                    }
                };
            }
            match cur {
                Some((st, stage, mv, rd)) => {
                    let st_prev = self.ev_state[fi].attack;
                    let released = matches!(st_prev, Some((s, true)) if s == st);
                    if st_prev.map(|x| x.0) != Some(st) {
                        self.ev_state[fi].attack = Some((st, false));
                    }
                    if stage == 1 && !released {
                        self.ev_state[fi].attack = Some((st, true));
                        let (ts, te) = self.posed.borrow().get(&name).map(|p| (p.tracer.cur_start, p.tracer.cur_end)).unwrap_or_default();
                        self.combat.emit_event(json!({"kind": "release", "who": name, "trace_start": [ts.x, ts.y, ts.z],
                            "trace_end": [te.x, te.y, te.z], "stab": mv == 2 || mv == 3, "release_duration": rd,
                            "yell_emitted": self.ev_state[fi].yelled == Some(st)}));
                    }
                }
                None => self.ev_state[fi].attack = None,
            }
            // foot notifies
            let notes = match self.fanim.get_mut(fi).and_then(|f| f.lower.as_mut()) {
                Some(l) => std::mem::take(&mut l.notifies),
                None => Vec::new(),
            };
            for n in notes {
                let foot = match n.as_str() {
                    "LeftFootLanded" => 0,
                    "RightFootLanded" => 1,
                    _ => continue,
                };
                let bone = if foot == 0 { "LeftFoot" } else { "RightFoot" };
                let pos = {
                    let posed = self.posed.borrow();
                    match (posed.get(&name), self.geo.skeleton.find(bone)) {
                        (Some(p), Some(b)) => self.geo.bone_world(p, b).loc,
                        _ => self.movers[fi].location,
                    }
                };
                let m = &self.movers[fi];
                let v = m.velocity;
                let comp = if m.current_floor.blocking_hit { m.current_floor.hit.component } else { None };
                // the floor hit's EPhysicalSurface (OnFootstep's SurfaceType) from the host's lookup at the floor point
                let fp = m.current_floor.hit.impact_point;
                let surface = match (comp, &self.surface_of) {
                    (Some(c), Some(f)) => f(c, fp),
                    _ => 0,
                };
                self.combat.emit_event(json!({"kind": "foot_landed", "who": name, "foot": foot, "foot_pos": [pos.x, pos.y, pos.z],
                    "speed": ((v.x * v.x + v.y * v.y) as f64).sqrt(), "crouched": m.is_crouched, "floor_component": comp, "surface": surface}));
            }
        }
    }
}

//! Per-fighter state snapshot as JSON: the exact object godot/tools/export_golden.gd `snap()` writes, so the golden
//! parity test compares field by field. Also a convenient read-only view for hosts (FFI `mh_world_state_json`).

use crate::combat::motion::MotionKind;
use crate::combat::World;
use serde_json::{json, Map, Value};

pub fn fighter(w: &World, fi: usize) -> Value {
    let f = &w.fighters[fi];
    let id = f.motion.expect("fighter without a motion");
    let m = w.m(fi, id);
    let mut d = json!({
        "kind": m.kind(), "stamina": f.stamina, "health": f.health, "dead": f.dead,
        "net": [f.net.id, f.net.motion_type, f.net.param0, f.net.param1, f.net.param2, f.net.dynamic_param],
        "next_kick": f.next_kick_time, "next_attack": f.next_attack_time, "next_stun": f.next_available_stun_time,
        "next_regen": f.next_stamina_regen, "easy_parry": f.easy_parry_until_time, "wants_block": f.wants_block,
        "holding_block": f.holding_block, "weapon": f.weapon.as_ref().map(|x| x.id.clone()).unwrap_or_default(),
        "left": f.left_hand_path, "alt": f.alternate_mode, "last_chance": f.has_last_chance,
        "regen_ok": f.stamina_regenerable, "mr": w.motion_movement_restriction(fi, id),
        "start": m.start_time, "end": m.end_time, "leave": m.leave_time, "flinchable": m.b_is_flinchable,
        "can_attack": m.b_can_attack, "can_block": m.b_can_block, "blocks_regen": m.b_blocks_regen,
        "cf": m.coming_from.map(|c| w.m(fi, c).kind()).unwrap_or(""),
        "last_attack": f.last_attack_motion.is_some(), "last_parry": f.last_parry_motion.is_some(),
        "ignore": f.tracer.actor_ignore_cache.iter().map(|i| w.name_of(*i)).collect::<Vec<_>>(),
        "length": f.tracer.length,
    });
    let o = d.as_object_mut().unwrap();
    let mut put = |extra: Value| {
        if let Value::Object(e) = extra {
            for (k, v) in e {
                o.insert(k, v);
            }
        }
    };
    match &m.k {
        MotionKind::Attack(a) => put(json!({
            "native": a.native, "type": a.ty, "move": a.mv, "stage": a.stage, "windup_end": a.windup_end,
            "release_end": a.release_end, "angle_target": a.angle_target, "lrn": a.last_release_norm,
            "lwn": a.last_windup_norm, "queued": a.b_has_queued_move, "queued_move": a.queued_move,
            "queued_angle": a.queued_angle, "hit": a.b_has_hit, "chambered": a.b_has_chambered,
            "considered_combo": a.b_has_considered_combo, "combo_from_miss": a.b_is_combo_from_miss,
            "riposte_ate_feint": a.b_riposte_ate_feint_input, "coming_from_move": a.coming_from_move,
            "ai_windup": a.ai.windup, "ai_release": a.ai.release, "ai_miss_recovery": a.ai.miss_recovery,
            "ai_stamina_drain": a.ai.stamina_drain, "ai_miss_stamina_cost": a.ai.miss_stamina_cost,
            "ai_damage": a.ai.damage.iter().map(|x| *x as f64).collect::<Vec<_>>(), "first_hit": a.first_hit_time,
            "early_release": a.early_release, "early_release_tf": a.early_release_tf,
            "in_early_release": w.attack_is_in_early_release(fi, id),
            "hit_actors": a.hit_actors.iter().map(|i| w.name_of(*i)).collect::<Vec<_>>(),
            "has_last_trace": a.b_has_last_trace, "air_kick": a.b_is_air_kick, "windup_curve": a.windup_curve,
            "release_curve": a.release_curve, "bounce_montage": a.bounce_montage, "hit_friendly": a.b_has_hit_friendly,
        })),
        MotionKind::Parry(p) => put(json!({
            "stage": p.stage, "blocks": p.total_blocks, "parry_end": p.parry_end,
            "recovery_start": p.recovery_start_time, "riposte_window_start": p.riposte_window_start,
            "queued": p.b_has_queued_move, "queued_move": p.queued_move, "queued_time": p.queued_move_time,
            "queued_angle": p.queued_angle, "block_type": p.block_type, "shield_wall": p.b_is_shield_wall,
            "holdable": p.b_is_block_holdable, "miss_parry": p.b_is_miss_parry,
            "detected": p.b_detected_any_non_friendly_attack, "requested_drop": p.b_requested_drop,
            "parry_up_time": p.parry_up_time, "parry_recovery_time": p.parry_recovery_time,
            "min_held": p.minimum_held_parry_time, "non_held_ext": p.non_held_parry_extension_time,
            "recovery_type": p.recovery_type, "cum_drain": p.cumulative_stamina_drain,
            "last_blocked_move": p.last_blocked_move,
        })),
        MotionKind::Feinted(x) => put(json!({
            "lockout": x.lock_out_time, "strike_lockout": x.strike_lockout, "stab_lockout": x.stab_lockout,
            "feint_type": x.feint_type, "from_move": x.from_move, "queued": x.b_has_queued_move,
            "queued_move": x.queued_move, "queued_angle": x.queued_angle, "queue_execute": x.queue_execute_time,
        })),
        MotionKind::Blocked(b) => put(json!({
            "reason": b.reason, "flags": [b.b_is_stun, b.b_is_disarm, b.b_is_ranged, b.b_is_cancel, b.b_party_flag,
                b.b_requires_self_block_event, b.b_clash_on_parry], "from_move": b.from_move,
            "queued": b.b_has_queued_move, "queued_move": b.queued_move, "queued_angle": b.queued_angle,
            "stop_anim_fade": b.stop_anim_fade, "kick_rate": b.kick_hit_stop_anim_rate,
            "bounce_montage": b.bounce_montage, "bounce_additive": b.bounce_additive,
            "bounce_pos": b.bounce_anim_position, "faded": b.b_has_faded_out_procedural,
            "release_bounce": b.b_do_release_bounce_procedural, "orig_mr": b.original_movement_restriction,
        })),
        MotionKind::Flinch(x) => put(json!({
            "speed_factor": x.speed_factor, "foley": x.b_has_played_foley, "ik_speed": x.offhand_ik_change_speed,
            "atmos": x.b_disables_atmospherics,
        })),
        MotionKind::Stun(s) => put(json!({"will_disarm": s.b_will_disarm, "direction": s.direction})),
        MotionKind::Disarmed(s) => put(json!({"direction": s.direction})),
        MotionKind::Climbing(c) => put(json!({"slow": c.b_is_slow_climb, "params": c.params})),
        MotionKind::Vehicle(v) => put(json!({"leaving": v.leaving, "param": v.param})),
        MotionKind::Ranged(r) => put(json!({"draw_time": r.draw_time, "fired": r.has_fired, "reload_time": r.reload_time})),
        // fp-anim r3: UEquipmentModeSwitchMotion (mode_switch_motion.gd trace_fields)
        MotionKind::ModeSwitch(x) => put(json!({"to_alt": x.b_is_switching_to_alt, "finished": x.b_has_finished_switch})),
        MotionKind::Idle => {}
    }
    d
}

/// One golden row: {n, t, f: {name: snapshot}, ev: [events since], log: [lines since], mem}
pub fn row(w: &World, hits0: usize, log0: usize) -> Value {
    let mut fs = Map::new();
    for fi in 0..w.fighters.len() {
        fs.insert(w.fighters[fi].name.clone(), fighter(w, fi));
    }
    json!({
        "n": w.tick_n, "t": w.now, "f": fs,
        "ev": w.hits[hits0..].iter().map(|m| Value::Object(m.clone())).collect::<Vec<_>>(),
        "log": w.events[log0..].to_vec(), "mem": w.attack_traces_memory.len(),
    })
}

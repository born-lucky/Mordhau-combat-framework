//! UBotBehaviorProfile (godot/game/ai/bot_behavior_profile.gd + the BotData.Profile record): a bot's skill numbers and
//! the per-motion random rolls drawn from them. Class data comes from the native ctor rva=0x144de70 + the
//! BOTBEHAVIOR_* Blueprint chain (the host reads it; serde record here); each bot owns a copy that Randomize and
//! RerollRandomInstanceValues write into. Field offsets: extract/native/types/UBotBehaviorProfile.h.

use crate::consts::bot as K;
use mordhau_core::ue::{clampf, CrtRand};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Profile {
    // class data
    pub ignore_enemies_with_ally_count: i64,         // +0x44
    pub b_prefers_alt_mode: bool,                    // +0x5c
    pub back_off_factor_during_defense_min_max: [f32; 2], // +0x60
    pub base_attack_hesitance_time: f64,             // +0x68
    pub attack_hesitance_variance: f64,              // +0x6c
    pub footwork_instead_of_parry_probability: f64,  // +0x70
    pub footwork_with_crouch_probability: f64,       // +0x74
    pub parry_timing_variance: [f32; 2],             // +0x78
    pub perfect_parry_probability: f64,              // +0x80
    pub feint_timing_variance: f64,                  // +0x84
    pub fall_for_feint_probability: f64,             // +0x88
    pub out_of_range_feint_probability: f64,         // +0x8c
    pub combo_probability: f64,                      // +0x90
    pub drag_probability: f64,                       // +0x94
    pub accel_probability: f64,                      // +0x98
    pub chamber_probability: f64,                    // +0x9c
    pub morph_probability: f64,                      // +0xa0
    pub gamble_probability: f64,                     // +0xa4
    pub feint_probability: f64,                      // +0xa8
    pub riposte_probability: f64,                    // +0xac
    pub brawl_probability: f64,                      // +0xb0
    pub max_turn_rate: f64,                          // +0xb4
    pub max_look_up_rate: f64,                       // +0xb8
    // instance rolls (RerollRandomInstanceValues)
    pub random_2d_unit_vector: [f32; 2],             // +0xbc
    pub will_brawl: bool,                            // +0xc4
    pub will_riposte: bool,                          // +0xc6
    pub back_off_factor_during_defense: f64,         // +0xc8
    pub will_feint: bool,                            // +0xcc
    pub will_out_of_range_feint: bool,               // +0xcd
    pub will_gamble: bool,                           // +0xce
    pub will_morph: bool,                            // +0xcf
    pub will_chamber: bool,                          // +0xd0
    pub will_accel: bool,                            // +0xd1
    pub will_drag: bool,                             // +0xd2
    pub will_combo: bool,                            // +0xd3
    pub will_fall_for_feint: bool,                   // +0xd4
    pub feint_timing_random: f64,                    // +0xd8
    pub will_perfect_parry: bool,                    // +0xdc
    pub parry_timing_random: f64,                    // +0xe0
    pub will_footwork: bool,                         // +0xe4
    pub will_footwork_with_crouch: bool,             // +0xe5
    pub attack_hesitance_random: f64,                // +0xf0
}

/// the controller's BehaviorProfile (+0x480): this bot's copy + LastFootworkingEnemyMotion
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BehaviorProfile {
    pub p: Profile,
    /// +0xe8 TWeakObjectPtr<UMordhauMotion> LastFootworkingEnemyMotion (the motion's identity; None = null)
    pub last_footworking_enemy_motion: Option<i64>,
}

impl BehaviorProfile {
    pub fn new(p: Profile) -> BehaviorProfile {
        BehaviorProfile { p, last_footworking_enemy_motion: None }
    }

    /// from UBotBehaviorProfile::RerollRandomInstanceValues rva=0x149e910. One rand() per line, in this order.
    /// c = 1/32767 (frand_scale); the unit vector's zero-length guard and the clamps are .rdata constants too.
    pub fn reroll(&mut self, rng: &mut CrtRand) {
        self.reroll_q(rng, |x| x);
    }

    /// reroll with the numeric model's rounding (`q`: binary32 in exe mode: every line below is ss arithmetic there;
    /// the reference computes in doubles)
    pub fn reroll_q(&mut self, rng: &mut CrtRand, q: fn(f64) -> f64) {
        let c = K::FRAND_SCALE;
        let p = &mut self.p;
        let x = q(q(q(rng.rand() as f64 * c) * 2.0) - K::PROFILE_ONE);
        let y = q(q(q(rng.rand() as f64 * c) * 2.0) - K::PROFILE_ONE);
        let l2 = q(q(y * y) + q(x * x));
        if l2 <= K::PROFILE_UNIT_VECTOR_MIN_SQ {
            p.random_2d_unit_vector = [0.0, 0.0];
        } else {
            let inv = q(1.0 / q(l2.sqrt()));
            p.random_2d_unit_vector = [(y * inv) as f32, (x * inv) as f32]; // X = second draw, Y = first draw
        }
        let fr = |rng: &mut CrtRand| rng.frand(K::FRAND_SCALE);
        p.will_brawl = fr(rng) < p.brawl_probability;
        p.will_riposte = fr(rng) < p.riposte_probability;
        p.will_feint = fr(rng) < p.feint_probability;
        p.will_gamble = fr(rng) < p.gamble_probability;
        p.will_morph = fr(rng) < p.morph_probability;
        p.will_chamber = fr(rng) < p.chamber_probability;
        p.will_accel = fr(rng) < p.accel_probability;
        p.will_drag = fr(rng) < p.drag_probability;
        p.will_combo = fr(rng) < p.combo_probability;
        p.will_fall_for_feint = fr(rng) < p.fall_for_feint_probability;
        p.will_out_of_range_feint = fr(rng) < p.out_of_range_feint_probability;
        p.will_perfect_parry = fr(rng) < p.perfect_parry_probability;
        self.last_footworking_enemy_motion = None; // null weak ptr
        p.will_footwork = fr(rng) < p.footwork_instead_of_parry_probability;
        p.will_footwork_with_crouch = fr(rng) < p.footwork_with_crouch_probability;
        p.feint_timing_random = q(q(rng.rand() as f64 * p.feint_timing_variance) * c);
        let ptv = p.parry_timing_variance;
        p.parry_timing_random = q(q(q(ptv[1] as f64 - ptv[0] as f64) * clampf(fr(rng), 0.0, 1.0)) + ptv[0] as f64);
        let hv = p.attack_hesitance_variance;
        let hr = q(q(hv * rng.rand() as f64) * c);
        p.attack_hesitance_random = q(q(hv + p.base_attack_hesitance_time) - q(hr + hr));
        let bo = p.back_off_factor_during_defense_min_max;
        p.back_off_factor_during_defense = q(q(q(bo[1] as f64 - bo[0] as f64) * clampf(fr(rng), 0.0, 1.0)) + bo[0] as f64);
    }

    /// from UBotBehaviorProfile::Randomize rva=0x149c190 (a BotProfile with bRandomizeBehavior). Draw order as
    /// decompiled. randomize_cube_scale = 1/32767^3 (cube of a draw), c = 1/32767.
    pub fn randomize(&mut self, rng: &mut CrtRand) {
        let c3 = K::RANDOMIZE_CUBE_SCALE;
        let one = K::PROFILE_ONE;
        let cube = |rng: &mut CrtRand| {
            let r = rng.rand() as f64;
            r * r * r * c3
        };
        let fr = |rng: &mut CrtRand| rng.frand(K::FRAND_SCALE);
        let p = &mut self.p;
        p.ignore_enemies_with_ally_count = ((rng.rand() as f64 * K::RANDOMIZE_ALLY_COUNT_SCALE) as i64).min(2) + 1;
        let t = rng.rand() as f64 * K::FRAND_SCALE;
        p.b_prefers_alt_mode = ((t + t) as i64).min(1) == 1;
        p.combo_probability = one - cube(rng);
        p.base_attack_hesitance_time = cube(rng);
        p.attack_hesitance_variance = cube(rng);
        p.fall_for_feint_probability = fr(rng);
        p.brawl_probability = fr(rng);
        p.riposte_probability = one - cube(rng);
        p.feint_probability = fr(rng);
        p.gamble_probability = fr(rng);
        let a = rng.rand() as f64;
        let bb = rng.rand() as f64;
        p.parry_timing_variance = [(bb * bb * bb * c3) as f32, (a * a * a * c3) as f32]; // Y = first draw, X = second
        p.feint_timing_variance = fr(rng);
        p.drag_probability = fr(rng);
        p.accel_probability = fr(rng);
        p.chamber_probability = one - cube(rng);
        p.perfect_parry_probability = fr(rng);
        p.morph_probability = fr(rng);
        p.footwork_instead_of_parry_probability = fr(rng);
        p.footwork_with_crouch_probability = fr(rng);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // RerollRandomInstanceValues draw order (test_ai.gd test_reroll_thresholds_equal_profile): forced raw rand()
    // values decide each bWill* against the profile probability (strict <)
    #[test]
    fn reroll_thresholds() {
        let mut p = BehaviorProfile::new(Profile {
            feint_probability: 0.15000000596046448,
            brawl_probability: 0.10000000149011612,
            gamble_probability: 1.0,
            riposte_probability: 1.0,
            combo_probability: 1.0,
            fall_for_feint_probability: 0.699999988079071,
            out_of_range_feint_probability: 0.800000011920929,
            footwork_instead_of_parry_probability: 0.15000000596046448,
            footwork_with_crouch_probability: 1.0,
            parry_timing_variance: [-0.25, 0.4],
            ..Default::default()
        });
        let mut r = CrtRand::new(1);
        let (yes, no) = (0, 32767);
        r.forced = [16384, 16384, yes, no, 4915, yes, no, no, no, no, yes, 22936, 26213, no, 4914, yes, 10000, 24575, 1000, 30000]
            .into_iter()
            .collect();
        p.reroll(&mut r);
        let q = &p.p;
        assert!(q.will_brawl && !q.will_riposte && q.will_feint && q.will_gamble && q.will_combo);
        assert!(q.will_fall_for_feint && q.will_out_of_range_feint && q.will_footwork && q.will_footwork_with_crouch);
        assert!(!q.will_morph && !q.will_perfect_parry);
        assert_eq!(r.calls, 20);
    }
}

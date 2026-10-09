//! 2D UBlendSpace sampling from its cooked grid: the Rust port of godot/game/anim/blend_space.gd (rust-combat r4).
//!
//! A cooked BlendSpace keeps the editor's triangulation in GridSamples: one FEditorElement (Indices[3], Weights[3])
//! per grid point, X-major (index = XIndex * (GridNumY + 1) + YIndex). UBlendSpaceBase::GetNormalizedBlendInput
//! (VA 0x142ed2cc0): each axis clamped to [Min, Max], then (v - Min) * GridNum / (Max - Min);
//! UBlendSpace::GetGridSamplesFromBlendInput (VA 0x142ed1ed0): floor -> (x, y), remainder (rx, ry), the four grid
//! points weighted bilinearly; an index outside the array is an empty element.
//! fp-anim r1: TargetWeightInterpolationSpeedPerSec smoothing (`BsPlayer`) and the sync-group normalized time
//! (`SyncGroup`, animgraph.rs FighterAnim / lower.rs). Marker sync is not needed: the 1P / 3P locomotion clips carry no
//! AuthoredSyncMarkers (extract/json, e.g. 2H_Sword_Walk_1P), so the group falls back to normalized-time sync.

use serde_json::Value;

#[derive(Clone, Debug, Default)]
pub struct BlendSpace {
    pub path: String,
    /// SampleData: (animation package path, sample value X, Y)
    pub samples: Vec<(String, f64, f64)>,
    /// GridSamples: per grid point (sample indices, weights)
    pub grid: Vec<Vec<(i64, f64)>>,
    pub min: (f64, f64),
    pub max: (f64, f64),
    pub grid_n: (i64, i64),
    /// UBlendSpaceBase TargetWeightInterpolationSpeedPerSec (0 = no smoothing; BS_2H_Sword_Locomotion_1P 4.0,
    /// BS_LowerBodyLocomotion_1P 3.5)
    pub weight_speed: f64,
    /// bUseConstantInterpolation (+0xad, this engine build's field; UNCONFIRMED name-to-offset): InterpolateWeightOfSampleData
    /// steps the weights with FMath::FInterpConstantTo (0x1418aae00) when set, else FMath::FInterpTo (0x1418aae60)
    /// (disasm 0x142ed82d9..0x142ed82ea)
    pub constant_interp: bool,
}

/// a fixed array property as the dump writes it: either a JSON array or `Name`, `Name[1]`, `Name[2]`...
fn fixed_array(o: &Value, name: &str) -> Vec<Value> {
    if let Some(a) = o[name].as_array() {
        return a.clone();
    }
    let mut out = Vec::new();
    if !o[name].is_null() {
        out.push(o[name].clone());
    }
    let mut i = 1;
    while let Some(v) = o.get(format!("{name}[{i}]")) {
        out.push(v.clone());
        i += 1;
    }
    out
}

impl BlendSpace {
    /// from the BlendSpace export's properties (absent BlendParameters fields: FBlendParameter ctor Min 0, Max 100,
    /// GridNum 4)
    pub fn from_props(path: &str, p: &Value) -> BlendSpace {
        let samples = p["SampleData"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|s| {
                let a = &s["Animation"];
                let ap = a["ObjectPath"].as_str().or_else(|| a.as_str()).unwrap_or("");
                (crate::physics::strip(ap), s["SampleValue"]["X"].as_f64().unwrap_or(0.0), s["SampleValue"]["Y"].as_f64().unwrap_or(0.0))
            })
            .collect();
        let grid = p["GridSamples"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|g| {
                let (ix, ws) = (fixed_array(g, "Indices"), fixed_array(g, "Weights"));
                ix.iter().zip(ws.iter()).map(|(i, w)| (i.as_i64().unwrap_or(-1), w.as_f64().unwrap_or(0.0))).collect()
            })
            .collect();
        let bp = fixed_array(p, "BlendParameters");
        let axis = |i: usize| {
            let b = bp.get(i).cloned().unwrap_or(Value::Null);
            (b["Min"].as_f64().unwrap_or(0.0), b["Max"].as_f64().unwrap_or(100.0), b["GridNum"].as_i64().unwrap_or(4))
        };
        let (x, y) = (axis(0), axis(1));
        let weight_speed = p["TargetWeightInterpolationSpeedPerSec"].as_f64().unwrap_or(0.0);
        let constant_interp = p["bUseConstantInterpolation"].as_bool().unwrap_or(false);
        BlendSpace { path: path.to_string(), samples, grid, min: (x.0, y.0), max: (x.1, y.1), grid_n: (x.2.max(1), y.2.max(1)), weight_speed, constant_interp }
    }

    /// {sample index: weight} for input (x, y), as (index, weight) pairs in first-seen order
    pub fn weights(&self, x: f64, y: f64) -> Vec<(usize, f64)> {
        let vx = x.clamp(self.min.0, self.max.0);
        let vy = y.clamp(self.min.1, self.max.1);
        let gs = ((self.max.0 - self.min.0) / self.grid_n.0 as f64, (self.max.1 - self.min.1) / self.grid_n.1 as f64);
        let g = ((vx - self.min.0) / gs.0, (vy - self.min.1) / gs.1);
        let xi = (g.0.floor() as i64).min(self.grid_n.0 - 1);
        let yi = (g.1.floor() as i64).min(self.grid_n.1 - 1);
        let (rx, ry) = (g.0 - xi as f64, g.1 - yi as f64);
        let mut out: Vec<(usize, f64)> = Vec::new();
        for (dx, dy, c) in [(0, 0, (1.0 - rx) * (1.0 - ry)), (1, 0, rx * (1.0 - ry)), (0, 1, (1.0 - rx) * ry), (1, 1, rx * ry)] {
            if c <= 0.0 {
                continue;
            }
            let i = (xi + dx) * (self.grid_n.1 + 1) + (yi + dy);
            if i < 0 || i as usize >= self.grid.len() {
                continue;
            }
            for &(k, w) in &self.grid[i as usize] {
                if k < 0 {
                    continue;
                }
                match out.iter_mut().find(|o| o.0 == k as usize) {
                    Some(o) => o.1 += w * c,
                    None => out.push((k as usize, w * c)),
                }
            }
        }
        out
    }

    /// the clip every sample uses, "" when they differ (AdditiveMachine._single_clip)
    pub fn single_clip(&self) -> String {
        let first = self.samples.first().map(|s| s.0.clone()).unwrap_or_default();
        if self.samples.iter().all(|s| s.0 == first) { first } else { String::new() }
    }
}

/// FMath::FInterpConstantTo as compiled at 0x1418aae00 (scripts/ue_dis.py): d = target - cur; d^2 < 1e-8 -> target;
/// else cur + clamp(d, -dt speed, dt speed)
pub fn finterp_constant_to(cur: f64, target: f64, dt: f64, speed: f64) -> f64 {
    let d = target - cur;
    if d * d < 1e-8 {
        return target;
    }
    let step = speed * dt;
    cur + d.max(-step).min(step)
}

/// FMath::FInterpTo as compiled at 0x1418aae60: speed <= 0 or d^2 < 1e-8 -> target; else cur + d * clamp(dt speed, 0, 1)
pub fn finterp_to(cur: f64, target: f64, dt: f64, speed: f64) -> f64 {
    let d = target - cur;
    if !(speed > 0.0) || d * d < 1e-8 {
        return target;
    }
    cur + d * (dt * speed).clamp(0.0, 1.0)
}

/// A blend-space player's sample weights over time: UBlendSpaceBase::UpdateBlendSamples ->
/// InterpolateWeightOfSampleData (0x142ed8080; the interpolator choice and speed field disassembled, the list walk taken
/// from the engine source: UNCONFIRMED as compiled): with TargetWeightInterpolationSpeedPerSec (+0xa8) > 0 every sample
/// of the last frame moves toward its new target weight (0 when it left the triangle) at that speed (constant or
/// exponential, `constant_interp`), new samples rise from 0 the same way, samples at or below
/// ZERO_ANIMWEIGHT_THRESH (1e-5) drop out, and the list is normalized (NormalizeSampleDataWeight). Without a speed the
/// target weights are used as they are.
#[derive(Clone, Debug, Default)]
pub struct BsPlayer {
    pub weights: Vec<(usize, f64)>,
    pub started: bool,
}

impl BsPlayer {
    pub fn update(&mut self, bs: &BlendSpace, x: f64, y: f64, dt: f64) -> Vec<(usize, f64)> {
        let target = bs.weights(x, y);
        if !self.started || bs.weight_speed <= 0.0 || dt <= 0.0 {
            self.started = true;
            self.weights = target.clone();
            return target;
        }
        let interp = |c: f64, t: f64| if bs.constant_interp { finterp_constant_to(c, t, dt, bs.weight_speed) } else { finterp_to(c, t, dt, bs.weight_speed) };
        let mut out: Vec<(usize, f64)> = Vec::new();
        for &(k, w) in &self.weights {
            if w <= 1e-5 {
                continue;
            }
            let t = target.iter().find(|x| x.0 == k).map(|x| x.1).unwrap_or(0.0);
            let nw = interp(w, t);
            if nw > 1e-5 {
                out.push((k, nw));
            }
        }
        for &(k, t) in &target {
            if self.weights.iter().any(|x| x.0 == k && x.1 > 1e-5) {
                continue;
            }
            let nw = interp(0.0, t);
            if nw > 1e-5 {
                out.push((k, nw));
            }
        }
        let tot: f64 = out.iter().map(|x| x.1).sum();
        if tot > 1e-5 {
            for x in out.iter_mut() {
                x.1 /= tot;
            }
        } else {
            out = target;
        }
        self.weights = out.clone();
        out
    }
}

/// The anim instance's sync group "Locomotion" (AB_MordhauCharacterAnimation: BlendSpacePlayer_1 / _2 / _3 and
/// SequencePlayer_4 / _5 CanBeLeader, BlendSpacePlayer_29 / _30 AlwaysFollower; GroupScope Local): one normalized
/// time the leader advances by dt * PlayRate / its length (UBlendSpaceBase::TickAssetPlayer: the weighted sample length
/// GetAnimationLengthFromSampleData = sum(weight * SequenceLength / RateScale); a sequence: its length / RateScale),
/// and the followers sample every clip at normalized time x the clip's length. Hand-over CONFIRMED (fp-anim r3):
/// UBlendSpaceBase::TickAssetPlayer 0x142ee46a0 writes the context's AnimLengthRatio into a follower's own accumulator
/// every tick (0x142ee528c) and a leader advances from its accumulator (0x142ee4fc1 / 0x142ee5141), so one shared
/// position is equivalent; the leader choice (highest weight) is the engine source's.
#[derive(Clone, Debug, Default)]
pub struct SyncGroup {
    pub norm: f64,
}

impl SyncGroup {
    pub fn advance(&mut self, dt: f64, rate: f64, length: f64) {
        if length > 1e-6 && dt > 0.0 {
            self.norm = (self.norm + dt * rate / length).rem_euclid(1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_weights_bilinear() {
        // 1 x 1 grid over [0, 1]^2; four points each holding its own sample
        let p = serde_json::json!({
            "SampleData": [{"Animation": "a"}, {"Animation": "b"}, {"Animation": "c"}, {"Animation": "d"}],
            "GridSamples": [{"Indices": 0, "Weights": 1.0}, {"Indices": 1, "Weights": 1.0}, {"Indices": 2, "Weights": 1.0}, {"Indices": 3, "Weights": 1.0}],
            "BlendParameters": {"Min": 0.0, "Max": 1.0, "GridNum": 1},
            "BlendParameters[1]": {"Min": 0.0, "Max": 1.0, "GridNum": 1}
        });
        let b = BlendSpace::from_props("x", &p);
        let w = b.weights(0.25, 0.5);
        let get = |k: usize| w.iter().find(|x| x.0 == k).map(|x| x.1).unwrap_or(0.0);
        // point index = X * (GridNumY + 1) + Y: (0,0)=0, (0,1)=1, (1,0)=2, (1,1)=3
        assert!((get(0) - 0.375).abs() < 1e-12 && (get(1) - 0.375).abs() < 1e-12 && (get(2) - 0.125).abs() < 1e-12 && (get(3) - 0.125).abs() < 1e-12);
        assert_eq!(b.single_clip(), "");
    }

    #[test]
    fn weights_move_at_the_interpolation_speed() {
        // two points on a 1-D axis (Y grid 1): idle at 0, walk at 1; speed 4 / s
        let p = serde_json::json!({
            "SampleData": [{"Animation": "idle"}, {"Animation": "walk"}],
            "GridSamples": [{"Indices": 0, "Weights": 1.0}, {"Indices": 1, "Weights": 1.0}],
            "BlendParameters": {"Min": 0.0, "Max": 1.0, "GridNum": 1},
            "BlendParameters[1]": {"Min": 0.0, "Max": 1.0, "GridNum": 1},
            "TargetWeightInterpolationSpeedPerSec": 4.0,
            "bUseConstantInterpolation": true
        });
        let b = BlendSpace::from_props("x", &p);
        // X 0 -> grid points (0,0)=0 and (0,1)=1 by Y
        let mut pl = BsPlayer::default();
        let w0 = pl.update(&b, 0.0, 0.0, 0.016);
        assert_eq!(w0, vec![(0, 1.0)]);
        // jump to walk: after 0.1 s walk has risen by 0.4 (and idle fallen by 0.4)
        let w = pl.update(&b, 0.0, 1.0, 0.1);
        let get = |w: &[(usize, f64)], k: usize| w.iter().find(|x| x.0 == k).map(|x| x.1).unwrap_or(0.0);
        assert!((get(&w, 1) - 0.4).abs() < 1e-9 && (get(&w, 0) - 0.6).abs() < 1e-9, "{w:?}");
        let w = pl.update(&b, 0.0, 1.0, 0.2);
        assert!((get(&w, 1) - 1.0).abs() < 1e-9 && get(&w, 0) == 0.0, "{w:?}");
    }

    #[test]
    fn sync_group_wraps_normalized_time() {
        let mut g = SyncGroup::default();
        g.advance(0.5, 1.0, 1.0);
        g.advance(0.75, 1.0, 1.0);
        assert!((g.norm - 0.25).abs() < 1e-12);
        g.advance(0.25, 2.0, 2.0);
        assert!((g.norm - 0.5).abs() < 1e-12);
    }
}

//! mh-parity: demo parity for the Rust core (state/rust_brief.md; the GDScript pipeline is docs/PARITY.md).
//!
//! A 1:1 port of the simulation side of the parity pipeline:
//!   - `Prober::run`          = godot/tests/parity/probes.gd `run` (a scripted World run -> per-transition rows)
//!   - the metric probes      = probes.gd attack_timeline / feint / morph / combo / riposte / parry / blocked / flinch,
//!                              incl. combat r13's `parry_attacker` mask rule
//!   - `Prober::probe_weapon` = godot/tests/parity/replay.gd `probe_weapon`
//!   - `Prober::replay`       = replay.gd `replay` mode (recorded input sequences -> rows)
//! The demo statistics and verdicts stay in scripts/parity/compare.py (one implementation for every backend):
//! `compare.py --backend rust-compat|rust-exe` runs the `mh-parity` binary instead of replay.gd and writes
//! state/parity/compare_rust_<mode>.json. Nothing here reads a constant: a metric is whatever the World does.
//!
//! Modes: `Compat` = World::new_reference (the GDScript reference bit for bit; the equivalence gate against the
//! GDScript compare.json), `Exe` = World::new (exe-exact f32 arithmetic and clock, combat/exe.rs; the scoreboard
//! against the real demos).
//!
//! Fighter order: the reference iterates the scenario's fighters Dictionary in insertion order; this crate keeps the
//! order it is given (`Vec`). serde_json (no preserve_order) sorts object keys, which equals the insertion order of
//! every scenario compare.py builds ({"A", "B"}, {"P", "X"}).

pub mod modhooks;
pub mod source;
pub mod traced;

use mordhau_core::combat::world::Input;
use mordhau_core::combat::{MotionKind, World};
use mordhau_core::data::Spec;
use mordhau_core::ue::{FVector, Xform};
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// probes.gd constants
pub const DT: f64 = 0.001;
pub const T0: f64 = 0.1;
pub const MAX_UNTIL: f64 = 10.0; // s: a run never simulates longer (demo scenarios are <= 8 s)
pub const MAX_ROWS: usize = 20000; // state changes per run
/// experimental-parry block offsets (first, step, count) s before the contact (rust-parity r6; a sampling grid, not a
/// game rule): covers the demos' block offsets 0..0.30 s (state/parity/parry_block_times.json)
pub const PARRY_BLOCK_GRID: (f64, f64, i64) = (0.0, 0.02, 19);
pub const LS: &str = "Mordhau/Content/Mordhau/Blueprints/Equipment/Weapons/Specific/TwoHandedSword/BP_Longsword";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// World::new_reference: the GDScript reference model
    Compat,
    /// World::new: exe-exact
    Exe,
}

/// Godot's snappedf: floor(v / step + 0.5) * step
pub fn snapped(v: f64, step: f64) -> f64 {
    (v / step + 0.5).floor() * step
}

/// One row: [t, who, kind, stage, type, move, windup_end, release_end, end, blocks, stamina, extra]
#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    /// the World tick of this row (not written to JSON): in exe mode the clock is UWorld's f32 running sum, so a row's
    /// time can sit a few ulp below the scheduled input time of the same tick; the probes compare ticks, not times
    pub n: i64,
    pub t: f64,
    pub who: String,
    pub kind: String,
    pub stage: String,
    pub typ: i64,
    pub mv: i64,
    pub we: f64,
    pub re: f64,
    pub end: f64,
    pub blocks: i64,
    pub stamina: i64,
    pub extra: f64,
}

impl Row {
    pub fn to_json(&self) -> Value {
        json!([self.t, self.who, self.kind, self.stage, self.typ, self.mv, self.we, self.re, self.end, self.blocks,
            self.stamina, self.extra])
    }
}

/// a NaN metric is written as null (Godot's JSON.stringify of NAN, as compare.py reads it)
pub fn num(x: f64) -> Value {
    if x.is_finite() {
        json!(x)
    } else {
        Value::Null
    }
}

/// tick of a probe time (probes.gd compares `r[0] >= t_from - 1e-9` on the reference's n * dt clock; every t_from it
/// passes is a DT-grid time or a row time, so this is the same test in compat mode and the tick-exact one in exe mode)
fn tick(t: f64) -> i64 {
    (t / DT).round() as i64
}

fn find<'a>(rows: &'a [Row], who: &str, kind: &str, stage: &str, typ: i64, t_from: f64) -> Option<&'a Row> {
    if t_from.is_nan() {
        return None; // `r[0] >= NAN` is false for every row in the reference
    }
    let n0 = tick(t_from);
    rows.iter().find(|r| {
        r.who == who && r.n >= n0 && r.kind == kind && (stage.is_empty() || r.stage == stage)
            && (typ == -99 || r.typ == typ)
    })
}
fn tt(r: Option<&Row>) -> f64 {
    r.map(|r| r.t).unwrap_or(f64::NAN)
}
fn stamina_before(rows: &[Row], who: &str, t: f64) -> i64 {
    let mut s = -1;
    for r in rows {
        if r.who == who && r.n < tick(t) {
            s = r.stamina;
        }
    }
    s
}

fn ev(t: f64, who: &str, kind: &str) -> Value {
    json!({"t": t, "who": who, "kind": kind})
}
fn ev_attack(t: f64, who: &str, mv: i64) -> Value {
    json!({"t": t, "who": who, "kind": "attack", "move": mv})
}
fn ev_contact(t: f64, who: &str, target: &str) -> Value {
    json!({"t": t, "who": who, "kind": "contact", "target": target})
}

#[derive(Clone, Debug)]
pub struct Timeline {
    pub windup: f64,
    pub windup_release: f64,
    pub miss_total: f64,
    pub miss_cost: i64,
}

// EAttackType values the probes look for (CombatEnums.AttackType: Regular 0, Riposte 1, Combo 2, PostClash 3, Morph 4)
const RIPOSTE: i64 = 1;
const COMBO: i64 = 2;
const MORPH: i64 = 4;
// EParryRecoveryType Miss (CombatEnums.ParryRecovery: Success 0, Fail 1, Miss 2)
const RECOVERY_MISS: i64 = 2;
const STAGE_NAMES: [&str; 3] = ["Windup", "Release", "Recovery"];
const PARRY_STAGE_NAMES: [&str; 2] = ["Parry", "Recovery"];

pub struct Prober {
    pub spec: Rc<Spec>,
    pub mode: Mode,
    /// the NoChamber mod's character script on a per-server layer (modhooks.rs; none = vanilla / pak defaults)
    pub hooks: modhooks::ModHooks,
    tl_cache: RefCell<HashMap<String, Timeline>>,
}

impl Prober {
    pub fn new(spec: Rc<Spec>, mode: Mode) -> Prober {
        Prober { spec, mode, hooks: modhooks::ModHooks::default(), tl_cache: RefCell::new(HashMap::new()) }
    }

    fn world(&self, dt: f64) -> World {
        match self.mode {
            Mode::Compat => World::new_reference(self.spec.clone(), dt),
            Mode::Exe => World::new(self.spec.clone(), dt),
        }
    }

    /// probes.gd `run`: fighters [(name, "<weapon>|<left item>")], inputs as replay.gd's JSON events
    pub fn run(&self, fighters: &[(String, String)], inputs: &[Value], until: f64, dt: f64) -> Vec<Row> {
        let mut w = self.world(dt);
        let until = until.min(MAX_UNTIL);
        for (nm, wp) in fighters {
            let mut parts = wp.splitn(2, '|');
            let right = parts.next().unwrap_or("");
            let left = parts.next().unwrap_or("");
            w.add_fighter(nm, right, left);
        }
        for e in inputs {
            let t = e["t"].as_f64().unwrap_or(0.0);
            let who = e["who"].as_str().unwrap_or("").to_string();
            let kind = e["kind"].as_str().unwrap_or("");
            if kind == "place" {
                // the fighter's actor frame (BlockCollider), for the miss-parry sweep
                let fi = w.fighter_index(&who).expect("place: no fighter");
                let o = FVector::new(e["x"].as_f64().unwrap_or(0.0) as f32, e["y"].as_f64().unwrap_or(0.0) as f32,
                    e["z"].as_f64().unwrap_or(0.0) as f32);
                w.fighters[fi].actor_xf = Some(Xform { rows: Xform::IDENTITY.rows, origin: o });
                continue;
            }
            let inp = match kind {
                "attack" => Input::Attack {
                    who,
                    mv: e["move"].as_f64().unwrap_or(0.0) as i64,
                    angle: e["angle"].as_f64().unwrap_or(0.0),
                },
                "feint" => Input::Feint { who },
                "parry" => Input::Parry { who, bt: e["bt"].as_f64().unwrap_or(0.0) as i64 },
                "release_block" => Input::ReleaseBlock { who },
                "contact" => Input::Contact {
                    who,
                    target: e["target"].as_str().unwrap_or("").to_string(),
                    bone: e["bone"].as_str().unwrap_or("Spine1").to_string(),
                },
                "switch_mode" => Input::SwitchMode { who },
                _ => continue,
            };
            w.at(t, inp);
        }
        let order: Vec<usize> = fighters.iter().map(|(n, _)| w.fighter_index(n).unwrap()).collect();
        let mut rows = Vec::new();
        let mut last: HashMap<usize, (String, String, i64, i64, i64, i64, u64)> = HashMap::new();
        let mut hs = modhooks::HookState::default();
        while w.now < until - w.dt * 0.5 {
            w.step();
            if self.hooks.any() {
                hs.after_step(&mut w, &self.hooks);
            }
            for &fi in &order {
                let f = &w.fighters[fi];
                let id = f.motion.expect("fighter without a motion");
                let m = w.m(fi, id);
                let (mut stage, mut typ, mut mv, mut we, mut re, mut blocks, mut extra) =
                    (String::new(), -1i64, -1i64, 0.0, 0.0, 0i64, 0.0);
                match &m.k {
                    MotionKind::Attack(a) => {
                        stage = STAGE_NAMES[a.stage as usize].to_string();
                        typ = a.ty;
                        mv = a.mv;
                        we = a.windup_end;
                        re = a.release_end;
                    }
                    MotionKind::Parry(p) => {
                        stage = PARRY_STAGE_NAMES[p.stage as usize].to_string();
                        blocks = p.total_blocks;
                        extra = p.recovery_type as f64;
                    }
                    MotionKind::Feinted(x) => extra = x.lock_out_time,
                    MotionKind::Blocked(b) => {
                        typ = b.reason;
                        extra = f.net.param2 as f64;
                    }
                    _ => {}
                }
                let key = (m.kind().to_string(), stage.clone(), typ, mv, blocks, f.stamina, m.start_time.to_bits());
                if last.get(&fi) != Some(&key) {
                    last.insert(fi, key);
                    rows.push(Row {
                        n: w.tick_n,
                        t: snapped(w.now, 1e-6),
                        who: f.name.clone(),
                        kind: m.kind().to_string(),
                        stage,
                        typ,
                        mv,
                        we: snapped(we, 1e-6),
                        re: snapped(re, 1e-6),
                        end: if m.end_time < 1e30 { snapped(m.end_time, 1e-6) } else { -1.0 },
                        blocks,
                        stamina: f.stamina,
                        extra,
                    });
                    if rows.len() >= MAX_ROWS {
                        eprintln!("probes.run: {} rows, stopped at t={:.3}", rows.len(), w.now);
                        return rows;
                    }
                }
            }
        }
        rows
    }

    fn run2(&self, fighters: &[(&str, &str)], inputs: Vec<Value>, until: f64) -> Vec<Row> {
        let f: Vec<(String, String)> = fighters.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
        self.run(&f, &inputs, until, DT)
    }

    // ---- attack timeline ------------------------------------------------------------------------------------------
    pub fn attack_timeline(&self, wp: &str, mv: i64) -> Timeline {
        let key = format!("{wp}|{mv}");
        if let Some(t) = self.tl_cache.borrow().get(&key) {
            return t.clone();
        }
        let rows = self.run2(&[("A", wp)], vec![ev_attack(T0, "A", mv)], 4.0);
        let rel = find(&rows, "A", "Attack", "Release", -99, 0.0);
        let rec = find(&rows, "A", "Attack", "Recovery", -99, 0.0);
        let idle = find(&rows, "A", "Idle", "", -99, T0 + DT);
        let tl = Timeline {
            windup: tt(rel) - T0,
            windup_release: tt(rec) - T0,
            miss_total: tt(idle) - T0,
            miss_cost: rec.map(|r| 100 - r.stamina).unwrap_or(-1),
        };
        self.tl_cache.borrow_mut().insert(key, tl.clone());
        tl
    }

    /// largest x in [lo, hi] (DT resolution) with accepted(x) true, assuming true below and false above
    fn last_true(&self, lo: f64, hi: f64, accepted: &dyn Fn(f64) -> bool) -> f64 {
        if !accepted(lo) {
            return f64::NAN;
        }
        let (mut a, mut b) = (lo, hi);
        while b - a > DT * 0.5 {
            let m = snapped((a + b) * 0.5, DT);
            if m <= a || m >= b {
                break;
            }
            if accepted(m) {
                a = m;
            } else {
                b = m;
            }
        }
        a
    }

    pub fn feint_deadline(&self, wp: &str, mv: i64) -> f64 {
        self.last_true(0.0, 1.6, &|x| {
            let rows = self.run2(&[("A", wp)], vec![ev_attack(T0, "A", mv), ev(T0 + x, "A", "feint")], T0 + x + 0.01);
            find(&rows, "A", "Feinted", "", -99, 0.0).is_some()
        })
    }

    /// feint at `lateness` (0 = at attack start, 1 = at the deadline) -> (lockout, cost)
    pub fn feint_lockout(&self, wp: &str, mv: i64, lateness: f64) -> (f64, i64) {
        let d = self.feint_deadline(wp, mv);
        let x = snapped(lateness.clamp(0.0, 1.0) * d, DT);
        let rows = self.run2(&[("A", wp)], vec![ev_attack(T0, "A", mv), ev(T0 + x, "A", "feint")], T0 + x + 2.0);
        let Some(fe) = find(&rows, "A", "Feinted", "", -99, 0.0) else { return (f64::NAN, -1) };
        let idle = find(&rows, "A", "Idle", "", -99, fe.t);
        (tt(idle) - fe.t, stamina_before(&rows, "A", fe.t) - fe.stamina)
    }

    fn morph_rows(&self, wp: &str, mv: i64, target: i64, x: f64, until_extra: f64) -> Vec<Row> {
        self.run2(&[("A", wp)], vec![ev_attack(T0, "A", mv), ev_attack(T0 + x, "A", target)], T0 + x + until_extra)
    }

    pub fn morph_deadline(&self, wp: &str, mv: i64, target: i64) -> f64 {
        self.last_true(0.16, 1.6, &|x| find(&self.morph_rows(wp, mv, target, x, 0.01), "A", "Attack", "", MORPH, 0.0).is_some())
    }

    /// -> (morph_earliest, morph_total, morph_cost)
    pub fn morph(&self, wp: &str, mv: i64, target: i64) -> (f64, f64, i64) {
        let er = self.morph_rows(wp, mv, target, 0.02, 0.4);
        let early = tt(find(&er, "A", "Attack", "", MORPH, 0.0)) - T0;
        let rows = self.morph_rows(wp, mv, target, 0.2, 4.0);
        let Some(mo) = find(&rows, "A", "Attack", "", MORPH, 0.0) else { return (early, f64::NAN, -1) };
        let idle = find(&rows, "A", "Idle", "", -99, mo.t);
        (early, tt(idle) - mo.t, stamina_before(&rows, "A", mo.t) - mo.stamina)
    }

    /// combo after a hit (scripted contact on dummy B early in the release) or a miss-combo
    pub fn combo_total(&self, wp: &str, mv: i64, hit: bool) -> f64 {
        let tl = self.attack_timeline(wp, mv);
        let rel_start = T0 + tl.windup;
        let mut inputs = vec![ev_attack(T0, "A", mv), ev_attack(snapped(rel_start + 0.03, DT), "A", mv)];
        if hit {
            inputs.push(ev_contact(snapped(rel_start + 0.02, DT), "A", "B"));
        }
        let rows = self.run2(&[("A", wp), ("B", LS)], inputs, T0 + 5.0);
        let Some(c) = find(&rows, "A", "Attack", "", COMBO, rel_start) else { return f64::NAN };
        tt(find(&rows, "A", "Idle", "", -99, c.t)) - c.t
    }

    /// B (wp) parries an attack from X (att's strike) -> (rows, tp, tc)
    fn parried(&self, wp: &str, extra: Vec<Value>, until: f64, att: &str) -> (Vec<Row>, f64, f64) {
        let tl = self.attack_timeline(att, 0);
        let rel_start = T0 + tl.windup;
        let tp = snapped(rel_start - 0.1, DT);
        let tc = snapped(rel_start + 0.05, DT);
        let mut inputs = vec![ev_attack(T0, "X", 0), ev(tp, "B", "parry"), ev_contact(tc, "X", "B")];
        inputs.extend(extra);
        (self.run2(&[("X", att), ("B", wp)], inputs, until), tp, tc)
    }

    /// probes.gd parry_attacker (combat r13): the Longsword when wp's parry can block it, else wp's own weapon.
    /// UParryMotion::CheckParry rva=0x164ed40 (decomp UParryMotion.cpp 1124-1129) blocks only when (WeaponPtr
    /// ParryMask (+0x19ac) | 2 when neither holdable nor shield wall) & the attack's Weapon AttackMask (+0x19a8) != 0.
    pub fn parry_attacker(&self, wp: &str) -> String {
        let mut parts = wp.splitn(2, '|');
        let right = parts.next().unwrap_or("");
        let left = parts.next().unwrap_or("");
        let left_is_weapon = !left.is_empty() && self.spec.equip(left).map(|e| e.is_weapon).unwrap_or(false);
        let pw = self.spec.weapon(if left_is_weapon { left } else { right }).expect("parry_attacker: weapon not in spec");
        let ls = self.spec.weapon(LS).expect("parry_attacker: Longsword not in spec");
        let mask = pw.parry_mask | if pw.b_is_parry_held { 0 } else { 2 };
        if mask & ls.attack_mask != 0 {
            LS.to_string()
        } else {
            right.to_string()
        }
    }

    pub fn riposte_window(&self, wp: &str) -> f64 {
        let att = self.parry_attacker(wp);
        let tb = self.parried(wp, vec![], 1.0, &att).2;
        self.last_true(0.0, 1.0, &|y| {
            let (rows, _, _) = self.parried(wp, vec![ev_attack(tb + y, "B", 0)], tb + y + 0.01, &att);
            find(&rows, "B", "Attack", "", RIPOSTE, 0.0).is_some()
        })
    }

    pub fn riposte_total(&self, wp: &str, mv: i64) -> f64 {
        let att = self.parry_attacker(wp);
        let (_, _, tc) = self.parried(wp, vec![], 0.01, &att);
        let (rows, _, _) = self.parried(wp, vec![ev_attack(tc + 0.01, "B", mv)], tc + 4.0, &att);
        let Some(ri) = find(&rows, "B", "Attack", "", RIPOSTE, 0.0) else { return f64::NAN };
        tt(find(&rows, "B", "Idle", "", -99, ri.t)) - ri.t
    }

    pub fn parry(&self, wp: &str) -> Map<String, Value> {
        let rows = self.run2(&[("B", wp)], vec![ev(T0, "B", "parry")], T0 + 3.0);
        let rec = find(&rows, "B", "Parry", "Recovery", -99, 0.0);
        let idle = find(&rows, "B", "Idle", "", -99, T0 + DT);
        let att = self.parry_attacker(wp);
        let (prows, tp, tc) = self.parried(wp, vec![], 3.0, &att);
        let recb = find(&prows, "B", "Parry", "Recovery", -99, 0.0);
        let blk = find(&prows, "B", "Parry", "Parry", -99, tc);
        let idleb = find(&prows, "B", "Idle", "", -99, tp + DT);
        let bl = find(&prows, "X", "Blocked", "", -99, 0.0);
        let idlex = find(&prows, "X", "Idle", "", -99, tt(bl));
        // miss-parry: X's unblocked strike crosses B's BlockCollider (B placed so the Longsword trace line lies inside
        // the box), no contact
        let tl = self.attack_timeline(LS, 0);
        let tpm = snapped(T0 + tl.windup_release - 0.1, DT);
        let mrows = self.run2(&[("X", LS), ("B", wp)], vec![ev_attack(T0, "X", 0),
            json!({"t": 0.0, "who": "B", "kind": "place", "x": -0.91, "y": 1.37, "z": 0.0}), ev(tpm, "B", "parry")], tpm + 3.0);
        let mut miss = f64::NAN;
        if let Some(mr) = find(&mrows, "B", "Parry", "Recovery", -99, 0.0) {
            if mr.extra as i64 == RECOVERY_MISS {
                miss = tt(find(&mrows, "B", "Idle", "", -99, mr.t)) - mr.t;
            }
        }
        let mut m = Map::new();
        m.insert("parry_up".into(), num(tt(rec) - T0));
        m.insert("parry_fail_recovery".into(), num(tt(idle) - tt(rec)));
        m.insert("parry_miss_recovery".into(), num(miss));
        m.insert("parry_up_block".into(), num(tt(recb) - tp));
        m.insert("parry_success_recovery".into(), num(tt(idleb) - tt(recb)));
        m.insert("block_drain".into(), json!(blk.map(|b| stamina_before(&prows, "B", tc) - b.stamina).unwrap_or(-1)));
        m.insert("blocked_p2".into(), num(bl.map(|b| b.extra).unwrap_or(-1.0)));
        m.insert("blocked_total".into(), num(tt(idlex) - tt(bl)));
        if self.hooks.exp_parry.is_some() {
            // experimental parry (modhooks.rs): the up time follows the block, so parry_up_block is a function of the
            // block's offset into the parry: sampled on PARRY_BLOCK_GRID (the contact stays at tc, the press moves),
            // as [block - parry start, recovery start - parry start] (the demo's tb - t0 and parry_up_block), and
            // interpolated per demo sample by compare.py (state/parity/parry_block_times.json)
            let mut grid = Vec::new();
            for k in 0..PARRY_BLOCK_GRID.2 {
                let off = snapped(PARRY_BLOCK_GRID.0 + k as f64 * PARRY_BLOCK_GRID.1, DT);
                let tp2 = snapped(tc - off, DT);
                let rows = self.run2(&[("X", att.as_str()), ("B", wp)], vec![ev_attack(T0, "X", 0), ev(tp2, "B", "parry"),
                    ev_contact(tc, "X", "B")], tc + 2.0);
                let ps = find(&rows, "B", "Parry", "Parry", -99, 0.0);
                let blk = rows.iter().find(|r| r.who == "B" && r.kind == "Parry" && r.blocks > 0);
                let rec = find(&rows, "B", "Parry", "Recovery", -99, 0.0);
                if let (Some(ps), Some(b), Some(rec)) = (ps, blk, rec) {
                    if b.t <= rec.t {
                        grid.push(json!([snapped(b.t - ps.t, 1e-6), snapped(rec.t - ps.t, 1e-6)]));
                    }
                }
            }
            m.insert("parry_up_block_at".into(), Value::Array(grid));
        }
        m
    }

    /// X (att) is parried by B (parrier): the attacker's Blocked byte and duration, and B's stamina drain
    pub fn blocked(&self, att: &str, parrier: &str) -> Map<String, Value> {
        let (rows, _, tc) = self.parried(parrier, vec![], 3.0, att);
        let bl = find(&rows, "X", "Blocked", "", -99, 0.0);
        let idlex = find(&rows, "X", "Idle", "", -99, tt(bl));
        let blk = find(&rows, "B", "Parry", "Parry", -99, tc);
        let mut m = Map::new();
        m.insert("blocked_p2".into(), num(bl.map(|b| b.extra).unwrap_or(-1.0)));
        m.insert("blocked_total".into(), num(tt(idlex) - tt(bl)));
        m.insert("block_drain".into(), json!(blk.map(|b| stamina_before(&rows, "B", tc) - b.stamina).unwrap_or(-1)));
        m
    }

    pub fn flinch_total(&self, wp: &str) -> f64 {
        let tl = self.attack_timeline(LS, 0);
        let tc = snapped(T0 + tl.windup + 0.05, DT);
        let rows = self.run2(&[("X", LS), ("B", wp)], vec![ev_attack(T0, "X", 0), ev_contact(tc, "X", "B")], tc + 3.0);
        let Some(fl) = find(&rows, "B", "Flinch", "", -99, 0.0) else { return f64::NAN };
        tt(find(&rows, "B", "Idle", "", -99, fl.t)) - fl.t
    }

    /// replay.gd probe_weapon
    pub fn probe_weapon(&self, wp: &str, lateness: f64, parriers: &[String]) -> Value {
        let mut r = Map::new();
        for mv in [0i64, 2] {
            let k = if mv == 0 { "strike" } else { "stab" };
            let tl = self.attack_timeline(wp, mv);
            if tl.windup.is_nan() {
                continue;
            }
            let mut m = Map::new();
            m.insert("windup".into(), num(tl.windup));
            m.insert("windup_release".into(), num(tl.windup_release));
            m.insert("miss_total".into(), num(tl.miss_total));
            m.insert("miss_cost".into(), json!(tl.miss_cost));
            m.insert("feint_deadline".into(), num(self.feint_deadline(wp, mv)));
            let (lo, cost) = self.feint_lockout(wp, mv, lateness);
            m.insert("feint_lockout".into(), num(lo));
            m.insert("feint_cost".into(), json!(cost));
            m.insert("feint_lockout_l0".into(), num(self.feint_lockout(wp, mv, 0.0).0));
            m.insert("feint_lockout_l1".into(), num(self.feint_lockout(wp, mv, 1.0).0));
            let tgt = if mv == 0 { 2 } else { 0 };
            m.insert("morph_deadline".into(), num(self.morph_deadline(wp, mv, tgt)));
            let (me, mt, mc) = self.morph(wp, mv, tgt);
            m.insert("morph_earliest".into(), num(me));
            m.insert("morph_total".into(), num(mt));
            m.insert("morph_cost".into(), json!(mc));
            m.insert("combo_total".into(), num(self.combo_total(wp, mv, true)));
            m.insert("misscombo_total".into(), num(self.combo_total(wp, mv, false)));
            m.insert("riposte_total".into(), num(self.riposte_total(wp, mv)));
            r.insert(k.into(), Value::Object(m));
        }
        let mut p = self.parry(wp);
        p.insert("riposte_window".into(), num(self.riposte_window(wp)));
        p.insert("flinch_total".into(), num(self.flinch_total(wp)));
        r.insert("parry".into(), Value::Object(p));
        let mut bl = Map::new();
        for pw in parriers {
            bl.insert(pw.clone(), Value::Object(self.blocked(wp, pw)));
        }
        r.insert("blocked_by".into(), Value::Object(bl));
        Value::Object(r)
    }

    /// replay.gd replay mode: one recorded scenario {fighters, inputs, until} -> rows
    pub fn replay(&self, sc: &Value, dt: f64) -> Vec<Row> {
        let fighters: Vec<(String, String)> = sc["fighters"]
            .as_object()
            .map(|o| o.iter().map(|(k, v)| (k.clone(), v.as_str().unwrap_or("").to_string())).collect())
            .unwrap_or_default();
        let inputs = sc["inputs"].as_array().cloned().unwrap_or_default();
        self.run(&fighters, &inputs, sc["until"].as_f64().unwrap_or(5.0), dt)
    }
}

/// Row-for-row diff of two compare.json files (rows keyed by server, weapon, kind, metric, other): status changes and
/// rewrite-value changes (|a - b| > tol, or null vs number). Returns a JSON summary.
pub fn diff_compare(a: &Value, b: &Value, tol: f64) -> Value {
    let key = |r: &Value| {
        format!("{}|{}|{}|{}|{}", r["server"].as_str().unwrap_or(""), r["weapon"].as_str().unwrap_or(""),
            r["kind"].as_str().unwrap_or(""), r["metric"].as_str().unwrap_or(""), r["other"].as_str().unwrap_or(""))
    };
    let rows_a: Vec<&Value> = a["rows"].as_array().map(|v| v.iter().collect()).unwrap_or_default();
    let rows_b: Vec<&Value> = b["rows"].as_array().map(|v| v.iter().collect()).unwrap_or_default();
    let mb: HashMap<String, &Value> = rows_b.iter().map(|r| (key(r), *r)).collect();
    let ka: std::collections::HashSet<String> = rows_a.iter().map(|r| key(r)).collect();
    let mut status_changed = Vec::new();
    let mut value_changed = Vec::new();
    let mut missing = Vec::new();
    let mut counts_a: HashMap<String, i64> = HashMap::new();
    let mut counts_b: HashMap<String, i64> = HashMap::new();
    for r in &rows_a {
        *counts_a.entry(r["status"].as_str().unwrap_or("").to_string()).or_default() += 1;
        let k = key(r);
        let Some(o) = mb.get(&k) else {
            missing.push(json!({"row": k, "in": "a"}));
            continue;
        };
        let (sa, sb) = (r["status"].as_str().unwrap_or(""), o["status"].as_str().unwrap_or(""));
        if sa != sb {
            status_changed.push(json!({"row": k, "a": sa, "b": sb, "rewrite_a": r["rewrite"], "rewrite_b": o["rewrite"]}));
        }
        let same = match (r["rewrite"].as_f64(), o["rewrite"].as_f64()) {
            (Some(x), Some(y)) => (x - y).abs() <= tol,
            _ => r["rewrite"] == o["rewrite"],
        };
        if !same {
            value_changed.push(json!({"row": k, "a": r["rewrite"], "b": o["rewrite"], "status_a": sa, "status_b": sb}));
        }
    }
    for r in &rows_b {
        *counts_b.entry(r["status"].as_str().unwrap_or("").to_string()).or_default() += 1;
        if !ka.contains(&key(r)) {
            missing.push(json!({"row": key(r), "in": "b"}));
        }
    }
    let flips = |from: &str, to: &str| {
        status_changed
            .iter()
            .filter(|c| c["a"].as_str().unwrap_or("").ends_with(from) && c["b"].as_str().unwrap_or("").ends_with(to))
            .count()
    };
    json!({
        "rows_a": rows_a.len(), "rows_b": rows_b.len(), "counts_a": counts_a, "counts_b": counts_b,
        "status_changed": status_changed.len(), "value_changed": value_changed.len(), "unmatched": missing.len(),
        "pass_to_fail": flips("PASS", "FAIL"), "fail_to_pass": flips("FAIL", "PASS"),
        "equivalent": status_changed.is_empty() && value_changed.is_empty() && missing.is_empty(),
        "status_changes": status_changed, "value_changes": value_changed, "unmatched_rows": missing,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapped_matches_godot() {
        assert_eq!(snapped(-0.0004, 0.001), 0.0);
        assert_eq!(snapped(0.0005, 0.001), 0.001);
        assert_eq!(snapped(0.5614, 0.001), 561.0 * 0.001);
    }

    #[test]
    fn diff_detects_flips() {
        let a = json!({"rows": [{"server": "vanilla", "weapon": "W", "kind": "parry", "metric": "m", "status": "PASS", "rewrite": 0.15}]});
        let b = json!({"rows": [{"server": "vanilla", "weapon": "W", "kind": "parry", "metric": "m", "status": "FAIL", "rewrite": 0.151}]});
        let d = diff_compare(&a, &b, 1e-6);
        assert_eq!(d["pass_to_fail"], 1);
        assert_eq!(d["value_changed"], 1);
        assert_eq!(d["equivalent"], false);
        assert_eq!(diff_compare(&a, &a, 1e-6)["equivalent"], true);
    }

    #[test]
    fn nan_is_null() {
        assert_eq!(num(f64::NAN), Value::Null);
        assert_eq!(num(0.5), json!(0.5));
    }
}

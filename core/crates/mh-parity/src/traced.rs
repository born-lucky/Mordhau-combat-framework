//! Traced-hit parity (rust-parity r3): the contact-timing metrics through mh-sim's Sim with real weapon traces on the
//! posed character physics asset, instead of the scripted contacts of the probes (lib.rs). This exercises hitboxes +
//! animation: the attacker's pose comes from mh-sim's animation stand-in (`mh_sim::anim::ClipPoser`: the attack
//! motion's Animation clip at rate 1 from StartTime; UNCONFIRMED until rust-combat ports motion_anim.gd's windup /
//! release normalized-time mapping into mh-sim, then this module picks it up through the same Sim), the defender
//! stands in the reference pose, and the hits come from AMordhauWeapon::SampleTracers rva=0x163c430 on the posed
//! skeleton (mh-sim trace.rs) through the same combat rules.
//!
//! Metrics (one Sim per weapon, move and distance; World clock = the exe's f32 TimeSeconds):
//!   contact_time   earliest hit after the attack starts, over defender distances 50..250 cm (the demos' contact_time
//!                  row is an edge_min: its p01 is the earliest observed contact, compared against this)
//!   contact_by_dist  the first hit time at each distance (null = the swing never reached the defender)
//!   parried_contact_time  the parry event of that blocked swing (attack-relative; the demos count the attacker's
//!                  Blocked start as a contact too)
//!   blocked_total  the defender parries (block pressed 0.1 s before WindupEnd) at the nearest reaching distance:
//!                  Blocked -> Idle of the attacker, with the block found by the traced blade (not a scripted contact)
//! Records: the spec matrix + paks (mh_sim::load: Spec, physics asset, skeleton, weapon sockets) and the character
//! movement records of mh-character (core/tests/golden/character/records.json).

use mh_character::{BoxWorld, CharacterSource, RecordsJson as CharRecords};
use mh_sim::{anim::ClipPoser, FighterDesc, Sim, SimInput};
use mordhau_core::ue::FVector;
use serde_json::{json, Map, Value};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

/// the stab aims swept per distance: (yaw offset deg, look-up deg; negative = down). A stab follows the player's camera
/// (yaw = the actor, look-up = the procedural spine bend, procedural.rs), and players aim it, while the probe's level,
/// straight stab passes at head height ~20 cm beside the defender's head (rust-parity r6 animdiag Poleaxe stab 130 cm:
/// trace end z 176..194, y 12..24 through the release). A sampling grid (UNCONFIRMED aim model), not a game rule.
/// (look-up -20 was dropped in r6: it touches the defender at the first release tick at polearm range, earlier than
/// any demo stab: Poleaxe 0.629 vs straight-contact median 0.830)
pub const STAB_AIMS: [(f32, f64); 6] = [(0.0, 0.0), (-8.0, 0.0), (8.0, 0.0), (0.0, -10.0), (-8.0, -10.0), (8.0, -10.0)];
/// the accel / drag sweep (rust-parity r7; a sampling grid, not a game rule): defender distances (cm), start yaws (deg;
/// the defender at +X), turn rates (deg/s requested from TURN_LEAD s before WindupEnd; the demos turn a median 20-40 deg
/// over the last 0.2 s before contact = 100-200 deg/s)
/// (r8: +/-300 deg/s added, capped at the attack's TurnCaps ~240: the demos' last-0.2 s turns reach 48-59 deg)
pub const TURN_SWEEP: ([f32; 3], [f32; 7], [f32; 7]) = ([90.0, 130.0, 170.0], [-60.0, -40.0, -20.0, 0.0, 20.0, 40.0, 60.0], [-300.0, -200.0, -100.0, 0.0, 100.0, 200.0, 300.0]);
pub const TURN_LEAD: f64 = 0.1;
/// s from spawn to the sweep's attack. Kept at one tick (r8), i.e. INSIDE the first 2 s where mh-sim holds the
/// turn-in-place LowerBodyRotationOffset at 0: that offset applies only while standing (|v| <= 5 cm/s,
/// NativeUpdateAnimation decomp 2113-2185), and 97% of the demo strike contacts are made while moving (Greatsword strike:
/// 7 of 262 at <= 5 cm/s, median 254 cm/s, ReplicatedMovement). A 2.1 s settle (standing, lag on) made every turning row
/// 0.04-0.09 s late (r8 run). A moving probe is the faithful alternative (not done: it changes the distance mid-swing).
pub const TURN_SETTLE: f64 = 0.001;
/// the window of the demo's turn measure (traced_conditioned.py TURN_WIN)
pub const TURN_WIN: f64 = 0.2;
pub const DISTANCES: [f32; 11] = [50.0, 70.0, 90.0, 110.0, 130.0, 150.0, 170.0, 190.0, 210.0, 230.0, 250.0];
const Z: f32 = 100.0; // capsule centre above the floor (mh-sim tests)

/// How the fighters are posed for the traces
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Anim {
    /// mh-sim's animation graph (rust-combat r3: animgraph.rs, the port of the reference's animation layer:
    /// montages, windup / release normalized-time mapping, blends) through `Sim::enable_anim`
    Graph,
    /// mh-sim's stand-in (anim.rs ClipPoser: the attack's clip at rate 1 from StartTime; UNCONFIRMED)
    Clip,
}

pub struct Traced {
    matrix: mh_spec::Spec,
    vfs: Arc<mh_pak::Vfs>,
    records: String,
    pub dt: f32,
    pub anim: Anim,
}

impl Traced {
    pub fn new(records_json: &Path, dt: f32, anim: Anim) -> Result<Traced, String> {
        let matrix = mh_spec::Spec::load(&mh_spec::Spec::default_dir(), false).map_err(|e| format!("spec matrix: {e:?}"))?;
        let vfs = Arc::new(mh_pak::Vfs::mount_default().map_err(|e| format!("paks: {e:?}"))?);
        let records = std::fs::read_to_string(records_json).map_err(|e| format!("{}: {e}", records_json.display()))?;
        Ok(Traced { matrix, vfs, records, dt, anim })
    }

    fn sim(&self, weapons: &[&str]) -> Result<Sim, String> {
        let rec = CharRecords(&self.records).load().map_err(|e| format!("character records: {e:?}"))?;
        let ld = mh_sim::load::load(&self.matrix, self.vfs.clone(), weapons)?;
        let mut floor = BoxWorld::new();
        floor.add_box(FVector::new(-20000.0, -20000.0, -200.0), FVector::new(20000.0, 20000.0, 0.0));
        Ok(Sim::new(Rc::new(ld.spec), Rc::new(ld.geo), rec, Box::new(floor), self.dt))
    }

    /// One swing of `wp` (move mv) at a defender `dist` cm in front; parry: the defender blocks before WindupEnd.
    /// -> (first hit time after the attack start, attacker Blocked -> Idle if it was blocked)
    /// `parry`: Some(t) = the defender presses block at attack-relative time t (rust-combat: a timed parry must be up
    /// when the posed blade arrives, and the blade has to cross the BlockCollider first, so it is timed to the
    /// unparried contact: contact - 0.12 s)
    fn swing(&self, poser: &mut ClipPoser, wp: &str, def_wp: &str, mv: i64, dist: f32, parry_at: Option<f64>) -> Result<(Option<f64>, Option<f64>), String> {
        self.swing3(poser, wp, def_wp, mv, dist, parry_at, (0.0, 0.0)).map(|r| (r.0, r.1))
    }

    /// swing + the first parry event (attack-relative): the attacker's Blocked start, which extract_events.py counts as
    /// a contact (`tc = a["t1"] if a["end"] == BLOCKED`)
    /// `aim` = (yaw offset deg, look-up deg) of the attacker (STAB_AIMS; (0, 0) = straight, level)
    fn swing3(&self, poser: &mut ClipPoser, wp: &str, def_wp: &str, mv: i64, dist: f32, parry_at: Option<f64>, aim: (f32, f64)) -> Result<(Option<f64>, Option<f64>, Option<f64>), String> {
        let parry = parry_at.is_some();
        let mut dp = def_wp.splitn(2, '|');
        let (def_w, def_l) = (dp.next().unwrap_or(""), dp.next().unwrap_or(""));
        let mut ws = vec![wp, def_w];
        if !def_l.is_empty() {
            ws.push(def_l);
        }
        let mut s = self.sim(&ws)?;
        let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: wp.into(), location: FVector::new(0.0, 0.0, Z), ..Default::default() });
        let b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: def_w.into(), left: def_l.into(), location: FVector::new(dist, 0.0, Z), yaw: 180.0, ..Default::default() });
        if self.anim == Anim::Graph {
            s.enable_anim(mh_sim::animgraph::AnimAssets::new(self.vfs.clone())?);
        }
        let steps = (2.5 / self.dt as f64) as i64;
        let (mut start, mut first_hit, mut blocked_at, mut blocked_total) = (None::<f64>, None::<f64>, None::<f64>, None::<f64>);
        let mut parried = false;
        let mut parry_t = None::<f64>;
        for n in 0..steps {
            let mut inp = Vec::new();
            if n == 0 && aim != (0.0, 0.0) {
                inp.push((a, SimInput { yaw: Some(aim.0), look_up: Some(aim.1), ..Default::default() }));
            }
            if n == 1 {
                inp.push((a, SimInput { attack: Some((mv, 0.0)), ..Default::default() }));
            }
            if parry && !parried {
                if let Some(st) = start {
                    if s.combat.now + self.dt as f64 >= st + parry_at.unwrap_or(0.0) {
                        inp.push((b, SimInput { parry: Some(0), ..Default::default() }));
                        parried = true;
                    }
                }
            }
            if self.anim == Anim::Clip {
                poser.pose(&mut s);
            }
            s.step(&inp);
            if start.is_none() {
                if let Some(m) = s.combat.cur_m(a).filter(|m| m.is_attack()) {
                    start = Some(m.start_time);
                }
            }
            let (ev, _) = s.drain();
            for e in ev {
                let k = e["kind"].as_str().unwrap_or("");
                if std::env::var("MH_PARITY_DEBUG").is_ok() {
                    eprintln!("{:.4} {:?}", s.combat.now, e);
                }
                if k == "parry" && parry_t.is_none() {
                    parry_t = Some(s.combat.now);
                }
                if (k == "hit" || k == "parry" || k == "chamber") && first_hit.is_none() {
                    first_hit = Some(s.combat.now);
                }
            }
            let kind = s.combat.cur_m(a).map(|m| m.kind()).unwrap_or("");
            if kind == "Blocked" && blocked_at.is_none() {
                blocked_at = Some(s.combat.now);
            }
            if kind == "Idle" && blocked_at.is_some() && blocked_total.is_none() {
                blocked_total = Some(s.combat.now - blocked_at.unwrap());
                break;
            }
            if !parry && first_hit.is_some() {
                break;
            }
        }
        Ok((first_hit.zip(start).map(|(h, st)| h - st), blocked_total, parry_t.zip(start).map(|(h, st)| h - st)))
    }

    /// One TURNING swing (rust-parity r7, the accel / drag sweep): the attacker starts at yaw `y0` (deg; the defender
    /// stands at +X, `dist` cm) and, from TURN_LEAD s before WindupEnd on, turns at `dir` deg/s; mh-sim
    /// rate-limits it with the attack's TurnCaps (rust-combat r4 combat/turncap.rs: AAdvancedCharacter::SetTurnCaps
    /// rva=0x14a15f0, Turn rva=0x14a8a90 clamp to TurnCapRemaining). Straight and level (stabs: look-up 0).
    /// -> (first contact after the attack start, yaw change over the last TURN_WIN s before it, the defender's bearing
    /// off the attacker's yaw at the contact): the same three numbers traced_conditioned.py reads off each demo contact
    fn turn_swing(&self, wp: &str, mv: i64, dist: f32, y0: f32, dir: f32) -> Result<Option<(f64, f64, f64)>, String> {
        let mut s = self.sim(&[wp, crate::LS])?;
        let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: wp.into(), location: FVector::new(0.0, 0.0, Z), yaw: y0, ..Default::default() });
        let _b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: crate::LS.into(), location: FVector::new(dist, 0.0, Z), yaw: 180.0, ..Default::default() });
        s.enable_anim(mh_sim::animgraph::AnimAssets::new(self.vfs.clone())?);
        // the attack starts after TURN_SETTLE s: mh-sim's turn-in-place LowerBodyRotationOffset (rust-combat r5,
        // NativeUpdateAnimation decomp 2113-2185) is held at 0 while the anim instance is younger than 2 s
        // MH_TURN_SETTLE (s) overrides it for the standing-lag variant (traced_scoreboard.py --settle)
        let settle = std::env::var("MH_TURN_SETTLE").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(TURN_SETTLE);
        let n_att = ((settle / self.dt as f64).round() as i64).max(1);
        let steps = n_att + (2.5 / self.dt as f64) as i64;
        let mut start = None::<f64>;
        let mut hist: Vec<(f64, f32)> = Vec::new();
        for n in 0..steps {
            let mut inp = Vec::new();
            if n == n_att {
                inp.push((a, SimInput { attack: Some((mv, 0.0)), ..Default::default() }));
            } else if n > n_att && dir != 0.0 {
                // turning from TURN_LEAD s before WindupEnd on, at `dir` deg/s (capped by the attack's TurnCaps)
                let we = s.combat.cur_m(a).and_then(|m| m.attack()).map(|x| x.windup_end);
                if we.map(|we| s.combat.now >= we - TURN_LEAD).unwrap_or(false) {
                    let y = s.yaw[a] + dir * self.dt;
                    inp.push((a, SimInput { yaw: Some(y), ..Default::default() }));
                }
            }
            s.step(&inp);
            if start.is_none() {
                if let Some(m) = s.combat.cur_m(a).filter(|m| m.is_attack()) {
                    start = Some(m.start_time);
                }
            }
            hist.push((s.combat.now, s.yaw[a]));
            let (ev, _) = s.drain();
            if ev.iter().any(|e| matches!(e["kind"].as_str(), Some("hit") | Some("parry") | Some("chamber"))) {
                let Some(st) = start else { return Ok(None) };
                let now = s.combat.now;
                let y1 = s.yaw[a] as f64;
                let y_old = hist.iter().rev().find(|h| h.0 <= now - TURN_WIN).map(|h| h.1 as f64).unwrap_or(y0 as f64);
                let ang = |x: f64| (x + 180.0).rem_euclid(360.0) - 180.0;
                return Ok(Some((now - st, ang(y1 - y_old), ang(0.0 - y1))));
            }
            if let Some(st) = start {
                if s.combat.now - st > 2.0 {
                    break;
                }
            }
        }
        Ok(None)
    }

    /// The accel / drag sweep (rust-parity r7): TURN_SWEEP over the defender distances / start yaws / turn directions
    /// -> [[dist, y0, dir, contact, turn, bearing], ...] per move (graph only; traced_conditioned.py matches each demo
    /// contact to its nearest swings in (distance, turn, bearing))
    pub fn turn_sweep(&self, wp: &str) -> Result<Value, String> {
        let mut r = Map::new();
        for (mv, k) in [(0i64, "strike"), (2i64, "stab")] {
            let mut rows = Vec::new();
            for &d in TURN_SWEEP.0.iter() {
                for &y0 in TURN_SWEEP.1.iter() {
                    for &dir in TURN_SWEEP.2.iter() {
                        if let Some((t, turn, brg)) = self.turn_swing(wp, mv, d, y0, dir)? {
                            rows.push(json!([d, y0, dir, t, turn, brg]));
                        }
                    }
                }
            }
            r.insert(k.into(), Value::Array(rows));
        }
        Ok(Value::Object(r))
    }

    /// The traced metrics of one weapon (see the module header)
    /// parriers: the defender weapons of the demo's blocked rows (blocked_by, as the scripted probes)
    pub fn probe_weapon(&self, wp: &str, parriers: &[String]) -> Result<Value, String> {
        let mut poser = ClipPoser::new(mh_assets::pak_source::PakSource::new(self.vfs.clone()));
        let mut r = Map::new();
        for (mv, k) in [(0i64, "strike"), (2i64, "stab")] {
            let mut by = Map::new();
            let mut best: Option<(f32, f64)> = None;
            let mut best_aim = (0.0f32, 0.0f64);
            let mut aim_by = Map::new();
            for d in DISTANCES {
                // strikes: straight and level (the demos' straight contacts, traced_conditioned.py); stabs are aimed:
                // the earliest contact over STAB_AIMS
                let aims: &[(f32, f64)] = if mv == 0 { &[(0.0, 0.0)] } else { &STAB_AIMS };
                let mut t = None::<f64>;
                let mut at_aim = (0.0f32, 0.0f64);
                for &aim in aims {
                    let (x, _, _) = self.swing3(&mut poser, wp, crate::LS, mv, d, None, aim)?;
                    if let Some(x) = x {
                        if t.map(|t| x < t).unwrap_or(true) {
                            t = Some(x);
                            at_aim = aim;
                        }
                    }
                }
                if t.is_some() && mv != 0 {
                    aim_by.insert(format!("{d}"), json!([at_aim.0, at_aim.1]));
                }
                by.insert(format!("{d}"), t.map(|x| json!(x)).unwrap_or(Value::Null));
                if let Some(t) = t {
                    if best.map(|b| t < b.1).unwrap_or(true) {
                        best = Some((d, t));
                        best_aim = at_aim;
                    }
                }
            }
            let mut m = Map::new();
            m.insert("contact_time".into(), best.map(|b| json!(b.1)).unwrap_or(Value::Null));
            m.insert("contact_dist".into(), best.map(|b| json!(b.0)).unwrap_or(Value::Null));
            m.insert("contact_by_dist".into(), Value::Object(by));
            if mv != 0 {
                m.insert("aim_by_dist".into(), Value::Object(aim_by));
            }
            if let Some((d, t)) = best {
                // the same swing parried (block pressed at contact - 0.12 s): Blocked -> Idle, and the parry event's
                // time = the parried contact (extract_events.py counts the attacker's Blocked start as a contact)
                let (_, bt, pt) = self.swing3(&mut poser, wp, crate::LS, mv, d, Some(t - 0.12), best_aim)?;
                m.insert("blocked_total".into(), bt.map(|x| json!(x)).unwrap_or(Value::Null));
                m.insert("parried_contact_time".into(), pt.map(|x| json!(x)).unwrap_or(Value::Null));
                if mv == 0 {
                    let mut by = Map::new();
                    for p in parriers {
                        let (_, bt) = self.swing(&mut poser, wp, p, mv, d, Some(t - 0.12))?;
                        by.insert(p.clone(), json!({"blocked_total": bt}));
                    }
                    r.insert("blocked_by".into(), Value::Object(by));
                }
            }
            r.insert(k.into(), Value::Object(m));
        }
        Ok(Value::Object(r))
    }
}

impl Traced {
    /// Animation diagnostics for one swing (rust-parity r6): per tick, the attacker's stage, the graph's montage state
    /// (MotionAnim position / blend-in / AutoBlend offset; DefaultSlot and FullBodySlot layers: sequence, sequence time,
    /// weight) and the weapon's TraceStart / TraceEnd sockets in world space (cm), plus the hits; `anim` picks the
    /// graph or the clip stand-in. A defender stands at `dist` cm in front (as probe_weapon).
    pub fn diag(&self, wp: &str, mv: i64, dist: f32, anim: Anim) -> Result<Value, String> {
        let mut poser = ClipPoser::new(mh_assets::pak_source::PakSource::new(self.vfs.clone()));
        let mut s = self.sim(&[wp, crate::LS])?;
        let a = s.add_fighter(&FighterDesc { name: "A".into(), weapon: wp.into(), location: FVector::new(0.0, 0.0, Z), ..Default::default() });
        let _b = s.add_fighter(&FighterDesc { name: "B".into(), weapon: crate::LS.into(), location: FVector::new(dist, 0.0, Z), yaw: 180.0, ..Default::default() });
        if anim == Anim::Graph {
            s.enable_anim(mh_sim::animgraph::AnimAssets::new(self.vfs.clone())?);
        }
        let mut rows = Vec::new();
        let mut start = None::<f64>;
        let steps = (1.6 / self.dt as f64) as i64;
        for n in 0..steps {
            let inp = if n == 1 { vec![(a, SimInput { attack: Some((mv, 0.0)), ..Default::default() })] } else { vec![] };
            if anim == Anim::Clip {
                poser.pose(&mut s);
            }
            s.step(&inp);
            let (ev, _) = s.drain();
            let hits: Vec<Value> = ev.into_iter().filter(|e| e["kind"] == "hit").map(|e| json!({"bone": e["bone"], "t": s.combat.now})).collect();
            let m = s.combat.cur_m(a);
            if start.is_none() {
                if let Some(m) = m.filter(|m| m.is_attack()) {
                    start = Some(m.start_time);
                }
            }
            let Some(st) = start else { continue };
            let stage = m.and_then(|m| m.attack()).map(|x| x.stage).unwrap_or(-1);
            let posed = s.posed.borrow();
            let geo = &s.geo;
            let g = &geo.weapons[wp];
            let (mut ts, mut te) = (Value::Null, Value::Null);
            if let (Some(x), Some(p0), Some(p1)) = (posed.get("A").and_then(|p| geo.weapon_world(p, g)), g.trace_start, g.trace_end) {
                let (p, q) = (x.apply(p0), x.apply(p1));
                ts = json!([p.x, p.y, p.z]);
                te = json!([q.x, q.y, q.z]);
            }
            drop(posed);
            let mut r = json!({"t": s.combat.now - st, "stage": stage, "start": ts, "end": te, "hits": hits});
            if anim == Anim::Graph {
                let fa = &s.fanim[a];
                let (ds, fs) = if let Some(asx) = &s.anim {
                    (fa.ma.montages.slot_layers(&s.combat.spec, &asx.slot_default, s.combat.now),
                     fa.ma.montages.slot_layers(&s.combat.spec, &asx.slot_full_body, s.combat.now))
                } else {
                    (vec![], vec![])
                };
                r["graph"] = json!({"position": fa.ma.position, "blend_in": fa.ma.blend_in, "offset": fa.ma.offset,
                    "auto": fa.ma.auto_used, "seq": fa.ma.seq, "default": ds, "full_body": fs});
            }
            rows.push(r);
            if s.combat.now - st > 1.5 {
                break;
            }
        }
        Ok(Value::Array(rows))
    }
}

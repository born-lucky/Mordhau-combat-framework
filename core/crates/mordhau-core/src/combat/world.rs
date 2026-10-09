//! CombatState: the deterministic combat simulation - a fixed-step world clock, fighters, input, hit processing, a
//! per-tick event stream (godot/game/combat/combat_state.gd).
//!
//! Tick order per step n (t = n * dt), as one engine frame runs it:
//!   1. calls (host / test hooks), then queued inputs (each controller is a tick prerequisite of its pawn:
//!      AController::AddPawnTickDependency exe 0x14301d3a0; UNCONFIRMED: that the input handlers run inside the
//!      controller's tick);
//!   2. every fighter's tick (TG_PrePhysics): AMordhauCharacter::LODTick order (Fighter tick, system.rs);
//!   3. every fighter's late tick (TG_PostPhysics, ULateTickComponent): PrepareForTracing while attacking, then
//!      tracing in Release, which processes hits;
//!   4. scripted contacts (a trace result handed in directly, processed like a traced hit).
//! Within one tick group the fighters run in insertion order. No Mordhau function orders one character's tick after
//! another's (AAdvancedCharacter::BeginPlay rva=0x1459110 adds prerequisites only inside one character), so the
//! insertion order is a simulation choice; it decides a same-tick trade (golden trade_order_ab / _ba).

use super::system::Fighter;
use crate::data::{ModeRules, Spec};
use crate::ue::FVector;
use serde_json::{json, Map, Value};
use std::collections::BTreeMap;
use std::rc::Rc;

/// One scheduled input / call (CombatState.schedule entries)
#[derive(Clone, Debug)]
pub enum Input {
    Attack { who: String, mv: i64, angle: f64 },
    Feint { who: String },
    Parry { who: String, bt: i64 },
    ReleaseBlock { who: String },
    SwitchMode { who: String },
    ToggleMode { who: String },
    /// the attacker's weapon reaches the defender's bone without geometry
    Contact { who: String, target: String, bone: String },
    /// host/test calls (CombatState.at): applied first in the tick, in order
    Call(Call),
}

#[derive(Clone, Debug)]
pub enum Call {
    SetStamina { who: String, v: i64 },
    SetHealth { who: String, v: i64 },
    SetArmorTierOverride { who: String, v: i64 },
    SetAirborne { who: String, v: bool },
    SetAirborneTime { who: String, v: f64 },
    SetLookUp { who: String, v: f64 },
    SetHoldingBlock { who: String, v: bool },
    SetPresetFlip { who: String, v: bool },
    SetArmDisabled { who: String, left: bool, v: bool },
    WeaponNoDrop { who: String },
    Blocked { who: String, reason: i64, flags: i64, time: f64 },
    RequestParry { who: String, bt: i64, ftp: bool },
    PresetAttack { who: String, mv: i64, angle: f64 },
    AttackNow { who: String, mv: i64, angle: f64 },
    EquipRight { who: String, weapon: String },
    /// UAttackMotion::SetHasHitIncludingCosmeticHit rva=0x163dff0 (a UFUNCTION; its only caller is the exec thunk at
    /// 0x141681a52): the current attack's bHasHitIncludingCosmeticHit (the hit shake is not modelled)
    SetHasHitIncludingCosmeticHit { who: String },
}

/// AMordhauGameState +0x530 AttackTracesMemory entry (types/FLineTraceMemoryEntry.h)
#[derive(Clone, Debug)]
pub struct TraceMemory {
    pub start: FVector,
    pub end: FVector,
    pub destroy_time: f64,
    pub owner: u32,
}

/// Numeric model of a World (docs/RUST_CORE.md "Exe-exact mode"; every difference listed in combat/exe.rs)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Precision {
    /// the shipped exe: every float field and operation binary32 (SSE ss ops), UWorld TimeSeconds an f32 accumulated
    /// by DeltaSeconds, exe-only rule details (combat/exe.rs). Default.
    Exe,
    /// the GDScript reference bit for bit (golden traces): f64 arithmetic, f32 only where the reference rounds,
    /// clock = tick_n * dt. Only constructible with the `reference_compat` feature.
    Reference,
}

/// The component a trace hit (FHitResult Component): a character's body shape on a bone, the character's BlockCollider
/// (AMordhauCharacter +0xf60 UBoxComponent), or a weapon's ClashCollider (AMordhauWeapon +0x1a50)
#[derive(Clone, Debug, PartialEq)]
pub enum HitComp {
    Body(String),
    BlockCollider,
    /// the ClashCollider of the weapon fighter `victim` holds
    Clash,
    /// the BlockCollider (+0x1a58) of the AMordhauShield fighter `victim` holds (BP_MordhauShield "BlockColliderBP")
    ShieldBlock,
}

/// One raw hit of a swept segment: fraction along it, the character (or the weapon's owner for Clash), the component,
/// and the segment (FHitResult TraceStart / TraceEnd, UE cm)
#[derive(Clone, Debug)]
pub struct RawHit {
    pub t: f64,
    pub victim: usize,
    pub comp: HitComp,
    pub trace_start: FVector,
    pub trace_end: FVector,
}

/// The BlockingHit of AMordhauWeapon::SampleTracers rva=0x163c430: the first tracer (in sampling order) whose
/// UWorld::LineTraceSingleByObjectType(ObjectTypesToQuery = 1 = ECC_WorldStatic, bReturnPhysicalMaterial) hits
/// (AMordhauWeapon::SampleTracer rva=0x163be10, decomp AMordhauWeapon.cpp 2240-2268; skipped once found)
#[derive(Clone, Debug, Default)]
pub struct WorldBlock {
    /// FHitResult ImpactPoint (UE cm)
    pub impact_point: FVector,
    /// FHitResult Actor is valid (level geometry: AStaticMeshActor / ALandscapeProxy / ABrush)
    pub actor: bool,
    /// the hit actor's bCanBeDamaged (AActor +0x59 bit 7). AStaticMeshActor ctor rva=0x3471d90 (call at
    /// 0x143471db7), ALandscapeProxy ctor rva=0x2982740 (0x142982901) and ABrush ctor rva=0x2f26dc0 (0x142f26e7a) call
    /// AActor::SetCanBeDamaged(false) (rva=0x2e3bac0)
    pub can_be_damaged: bool,
    /// host id of the hit component (mh-level body), for its EPhysicalSurface
    pub component: Option<u32>,
    /// FHitResult PhysMaterial -> UPhysicalMaterial SurfaceType (EPhysicalSurface); 0 = SurfaceType_Default
    pub surface: u8,
}

/// One AMordhauWeapon::SampleTracers rva=0x163c430 pass as a geometry host reports it: per tracer (sampling order) the
/// raw hits, CurrentTraceStart / CurrentTraceEnd, the BlockingHit with the index of the tracer that found it (every
/// hit of that tracer and the later ones carries bBlockingHit, SampleTracer 0x14163c2d0), and
/// LastObservedTraceDirection (+0x1c80, AMordhauWeapon::PrepareForTracing rva=0x16378e0)
#[derive(Clone, Debug, Default)]
pub struct TraceSample {
    pub segments: Vec<Vec<RawHit>>,
    pub cur_start: FVector,
    pub cur_end: FVector,
    pub blocking: Option<(usize, WorldBlock)>,
    pub last_dir: FVector,
}

/// UE-space body state a geometry host feeds each frame (exe geometry rules: CheckParry / CheckChamber /
/// CheckAttackParry / CheckClash / TestForwardParry read these)
#[derive(Clone, Copy, Debug, Default)]
pub struct FighterGeom {
    /// RootComponent ComponentToWorld translation (capsule centre)
    pub root: FVector,
    /// AMordhauCharacter CameraLocation1P / CameraRotation1P (pitch, yaw)
    pub camera_loc: FVector,
    pub camera_rot: (f32, f32),
    /// BlockCollider ComponentToWorld (rotation, translation; scale 1) and BoxExtent
    pub block_collider: crate::ue::FTransform,
    pub block_extent: FVector,
}

/// Weapon-trace geometry supplied by a host (mh-sim: UE-space body shapes of the character physics asset, weapon
/// sockets on the posed skeleton). The combat rules around it stay here: SampleTracer's ordering and filters
/// (nearest-first across actors, ignored actors, parrying hands) and the hit processing (melee_hit.rs). Without a
/// host the reference's Godot-space tracer (tracer.rs, Spec::body_boxes) runs.
pub trait TraceHost {
    /// AMordhauWeapon::PrepareForTracing rva=0x16378e0 for fighter `fi` (its late tick while an attack is current)
    fn prepare(&self, w: &World, fi: usize);
    /// AMordhauWeapon::SampleTracers rva=0x163c430 of fighter `fi`: for each swept segment, every raw hit (any order,
    /// any component: body shapes, enabled BlockColliders, enabled weapon ClashColliders); plus CurrentTraceStart /
    /// CurrentTraceEnd. No segments when the tracers are not both valid.
    fn sample(&self, w: &World, fi: usize) -> (Vec<Vec<RawHit>>, FVector, FVector);
    /// AMordhauWeapon::ResetTracers rva=0x163bb80 (bAreCurrentTracersInvalidated = 1), called by
    /// AMordhauWeapon::OnAttackStarted_Implementation rva=0x162eb40 from UAttackMotion::OnBegin_Implementation
    /// rva=0x162eda0 (exe mode only; exe.rs difference 9)
    fn reset(&self, _w: &World, _fi: usize) {}
    /// `sample` plus the world (BlockingHit) and LastObservedTraceDirection. Default: no world geometry.
    fn sample_ex(&self, w: &World, fi: usize) -> TraceSample {
        let (segments, cur_start, cur_end) = self.sample(w, fi);
        TraceSample { segments, cur_start, cur_end, blocking: None, last_dir: FVector::ZERO }
    }
    /// UWorld::LineTraceSingleByObjectType(ECC_WorldStatic) from `start` to `end`: the impact point (UE cm). Default:
    /// no world geometry.
    fn world_static_trace(&self, _start: FVector, _end: FVector) -> Option<FVector> {
        None
    }
}

pub struct World {
    pub spec: Rc<Spec>,
    pub(crate) equipment: super::equipment::EquipmentArena,
    /// optional weapon-trace geometry (TraceHost)
    pub trace_host: Option<Rc<dyn TraceHost>>,
    /// this tick's scheduled inputs between step_pre and step_post
    pending: Vec<Input>,
    pub precision: Precision,
    pub dt: f64,
    pub tick_n: i64,
    pub now: f64,
    pub fighters: Vec<Fighter>,
    pub schedule: BTreeMap<i64, Vec<Input>>,
    /// trace log lines ("%.3f %s"), CombatState.events
    pub events: Vec<String>,
    /// every emitted event object, in order (CombatState.hits / combat_event)
    pub hits: Vec<Map<String, Value>>,
    pub attack_traces_memory: Vec<TraceMemory>,
    /// CombatData.ModeRules of the running game mode; None = no game mode (its damage-chain branches are skipped)
    pub mode_rules: Option<ModeRules>,
    /// false: a client's copy of the world (mh-net); hits are processed on the authority only
    pub authority: bool,
    /// the replication hooks of a networked machine (mh-net); None = one machine, authority (net.rs)
    pub net: Option<Rc<dyn super::net::NetHooks>>,
    pub(crate) next_fighter_id: u32,
    /// fire requests of the ranged release motions, drained by the host (rangedmotion.rs)
    pub fires: Vec<super::rangedmotion::FireRequest>,
    /// the CRT rand() the draw motion's RandomValue comes from (OnRequestFire rva=0x155b3b0; UNCONFIRMED seed 1)
    pub rand: crate::ue::CrtRand,
}

impl World {
    /// An exe-exact world (Precision::Exe). `spec` should hold the exe's f32 record values (RecordsJson::exe /
    /// the mh-spec adapter); the step is rounded to f32 (UWorld::Tick DeltaSeconds is a float).
    pub fn set_trace_host(&mut self, h: Option<Rc<dyn TraceHost>>) {
        self.trace_host = h;
    }

    pub fn new(spec: Rc<Spec>, step_s: f64) -> World {
        let mut w = World::with_precision(spec, step_s as f32 as f64);
        w.precision = Precision::Exe;
        w
    }

    /// The GDScript reference reproduced bit for bit (golden traces).
    #[cfg(feature = "reference_compat")]
    pub fn new_reference(spec: Rc<Spec>, step_s: f64) -> World {
        World::with_precision(spec, step_s)
    }

    fn with_precision(spec: Rc<Spec>, step_s: f64) -> World {
        World {
            spec,
            equipment: Default::default(),
            trace_host: None,
            pending: Vec::new(),
            precision: Precision::Reference,
            dt: step_s,
            tick_n: 0,
            now: 0.0,
            fighters: Vec::new(),
            schedule: BTreeMap::new(),
            events: Vec::new(),
            hits: Vec::new(),
            attack_traces_memory: Vec::new(),
            mode_rules: None,
            authority: true,
            net: None,
            next_fighter_id: 0,
            fires: Vec::new(),
            rand: crate::ue::CrtRand::new(1),
        }
    }

    /// left_path: optional left-hand equipment (AMordhauCharacter LeftHandEquipment), "" = none
    pub fn add_fighter(&mut self, name: &str, weapon_path: &str, left_path: &str) -> usize {
        let fi = self.new_fighter(name, weapon_path, left_path);
        self.fighters[fi].creation_time = self.now; // AActor CreationTime: the pawn spawns now
        fi
    }

    /// A respawn: the game mode destroys the dead pawn and spawns a new character (AGameModeBase::RestartPlayer ->
    /// a fresh AMordhauCharacter), so the fighter at `fi` is replaced by a new one built exactly as `add_fighter`
    /// builds it (same name / weapon / left hand; a new pawn id, CreationTime = now). first-person r1 (test level).
    pub fn respawn_fighter(&mut self, fi: usize) {
        let (name, w, l, team) = {
            let f = &self.fighters[fi];
            (f.name.clone(), f.weapon_path.clone(), f.left_hand_path.clone(), f.team)
        };
        self.release_pawn_equipment(fi);
        let ni = self.new_fighter(&name, &w, &l);
        let nf = self.fighters.pop().expect("new fighter");
        debug_assert_eq!(ni, self.fighters.len());
        self.fighters[fi] = nf;
        self.fighters[fi].team = team;
        self.fighters[fi].creation_time = self.now;
    }

    /// CombatState.set_game_mode: the running mode's damage rules, as resolved in the spec ("<gm>|<gs>")
    pub fn set_game_mode(&mut self, game_mode_bp: &str, game_state_bp: &str) {
        let key = format!("{game_mode_bp}|{game_state_bp}");
        self.mode_rules = Some(self.spec.mode_rules.get(&key).cloned().unwrap_or_else(|| panic!("spec: no mode rules {key}")));
    }

    /// The rounding of one float operation in this world's model: binary32 in Exe, identity in Reference. Rules write
    /// `let q = self.qf();` then wrap every arithmetic result, `q(q(a * b) + c)`, as the exe's ss ops round each one
    /// (double rounding through f64 is exact for + - * / of f32 operands).
    #[inline]
    pub fn qf(&self) -> fn(f64) -> f64 {
        match self.precision {
            Precision::Exe => crate::ue::f32r,
            Precision::Reference => |x| x,
        }
    }
    #[inline]
    pub fn exe(&self) -> bool {
        self.precision == Precision::Exe
    }

    pub fn fighter_index(&self, name: &str) -> Option<usize> {
        self.fighters.iter().position(|f| f.name == name)
    }
    fn fi(&self, name: &str) -> usize {
        self.fighter_index(name).unwrap_or_else(|| panic!("no fighter {name}"))
    }

    /// from AMordhauGameState::IsFriendly_Implementation rva=0x159bc10 (characters, bIsFriendlyIfSelf false): same
    /// actor -> false; else both teams known (!= 0xff) and equal and the game state's bIsTeamMode. No game state -> false.
    pub fn is_friendly(&self, a: Option<usize>, b: Option<usize>) -> bool {
        let (Some(r), Some(a), Some(b)) = (&self.mode_rules, a, b) else { return false };
        if a == b {
            return false;
        }
        let ta = self.fighters[a].team & 0xff;
        let tb = self.fighters[b].team & 0xff;
        ta != 0xff && tb != 0xff && r.b_is_team_mode && ta == tb
    }

    // ---- live API (applied on the next step) ----
    pub fn input(&mut self, ev: Input) {
        self.push(self.tick_n + 1, ev);
    }

    // ---- scripted API ----
    /// CombatState.at_time: int(round(t / dt))
    pub fn at_time(&self, t: f64) -> i64 {
        (t / self.dt).round() as i64
    }
    pub fn push(&mut self, n: i64, ev: Input) {
        self.schedule.entry(n).or_default().push(ev);
    }
    pub fn at(&mut self, t: f64, ev: Input) {
        let n = self.at_time(t);
        self.push(n, ev);
    }

    pub fn run_until(&mut self, t_end: f64) {
        while self.now < t_end - self.dt * 0.5 {
            self.step();
        }
    }

    /// CombatState.step
    pub fn step(&mut self) {
        self.step_with(&mut |_| {});
    }

    /// CombatState.step with host calls appended to this tick's call phase (after the scheduled calls, before the
    /// inputs): mh-net's received server RPCs, which the reference schedules with CombatState.at(now + dt, callable)
    pub fn step_with(&mut self, calls: &mut dyn FnMut(&mut World)) {
        self.step_pre(calls);
        self.step_post();
    }

    /// The first half of a step, through every fighter's TG_PrePhysics tick (AMordhauCharacter::LODTick). A host that
    /// runs character movement (mh-sim: UCharacterMovementComponent ticks after the actor tick) calls step_pre, moves,
    /// feeds the new poses, then step_post (TG_PostPhysics late ticks + tracing, scripted contacts).
    pub fn step_pre(&mut self, calls: &mut dyn FnMut(&mut World)) {
        self.tick_n += 1;
        self.now = match self.precision {
            // UWorld::Tick: TimeSeconds += DeltaSeconds (both float) before the tick groups (mh-character exe.rs frame
            // order 0; the same world clock its ExeMovement keeps)
            Precision::Exe => (self.now as f32 + self.dt as f32) as f64,
            Precision::Reference => self.tick_n as f64 * self.dt,
        };
        let evs = self.schedule.remove(&self.tick_n).unwrap_or_default();
        for ev in &evs {
            if let Input::Call(c) = ev {
                self.apply_call(c);
            }
        }
        calls(self);
        for ev in &evs {
            if !matches!(ev, Input::Contact { .. } | Input::Call(_)) {
                self.apply_input(ev);
            }
        }
        self.prune_trace_memory();
        for fi in 0..self.fighters.len() {
            self.fighter_tick(fi, self.dt);
        }
        self.pending = evs;
    }

    /// The second half of a step (see step_pre)
    pub fn step_post(&mut self) {
        let evs = std::mem::take(&mut self.pending);
        for fi in 0..self.fighters.len() {
            self.fighter_late_tick(fi);
            if self.authority {
                self.trace_and_process(fi);
            }
        }
        for ev in &evs {
            if let Input::Contact { who, target, bone } = ev {
                if self.authority {
                    let (a, b) = (self.fi(who), self.fi(target));
                    self.contact(a, b, bone);
                }
            }
        }
        for fi in 0..self.fighters.len() {
            self.collect_motions(fi);
        }
    }

    /// AMordhauGameState::Tick rva=0x15ab790 (the loop over +0x530/+0x538): an entry whose DestroyTime (+0x18) is
    /// before now is removed. UNCONFIRMED: game-state tick before the characters'.
    fn prune_trace_memory(&mut self) {
        let now = self.now;
        self.attack_traces_memory.retain(|e| !(e.destroy_time < now));
    }

    fn apply_input(&mut self, ev: &Input) {
        let (who, kind) = match ev {
            Input::Attack { who, mv, angle } => {
                let fi = self.fi(who);
                self.request_attack(fi, *mv, *angle);
                (who, "attack")
            }
            Input::Feint { who } => {
                let fi = self.fi(who);
                self.request_feint(fi);
                (who, "feint")
            }
            Input::Parry { who, bt } => {
                let fi = self.fi(who);
                self.block_pressed(fi, *bt);
                (who, "parry")
            }
            Input::SwitchMode { who } => {
                let fi = self.fi(who);
                self.switch_mode(fi);
                (who, "switch_mode")
            }
            Input::ToggleMode { who } => {
                let fi = self.fi(who);
                self.toggle_weapon_mode(fi);
                (who, "toggle_mode")
            }
            Input::ReleaseBlock { who } => {
                let fi = self.fi(who);
                self.release_block(fi);
                (who, "release_block")
            }
            _ => return,
        };
        let s = format!("{who} input {kind}");
        self.trace_event(&s);
    }

    fn apply_call(&mut self, c: &Call) {
        match c {
            Call::SetStamina { who, v } => {
                let fi = self.fi(who);
                self.fighters[fi].stamina = *v;
            }
            Call::SetHealth { who, v } => {
                let fi = self.fi(who);
                self.fighters[fi].health = *v;
            }
            Call::SetArmorTierOverride { who, v } => {
                let fi = self.fi(who);
                self.fighters[fi].armor_tier_override = *v;
            }
            Call::SetAirborne { who, v } => {
                let fi = self.fi(who);
                self.fighters[fi].airborne = *v;
            }
            Call::SetAirborneTime { who, v } => {
                let fi = self.fi(who);
                self.fighters[fi].airborne_time = *v;
            }
            Call::SetLookUp { who, v } => {
                let fi = self.fi(who);
                self.fighters[fi].look_up_value = *v;
            }
            Call::SetHoldingBlock { who, v } => {
                let fi = self.fi(who);
                self.fighters[fi].holding_block = *v;
            }
            Call::SetPresetFlip { who, v } => {
                let fi = self.fi(who);
                self.fighters[fi].preset_flip = *v;
            }
            Call::SetArmDisabled { who, left, v } => {
                let fi = self.fi(who);
                if *left {
                    self.fighters[fi].b_is_left_arm_disabled = *v;
                } else {
                    self.fighters[fi].b_is_right_arm_disabled = *v;
                }
            }
            Call::WeaponNoDrop { who } => {
                let fi = self.fi(who);
                self.set_weapon_no_drop(fi);
            }
            Call::Blocked { who, reason, flags, time } => {
                let fi = self.fi(who);
                self.assign_net_blocked(fi, *reason, *flags, *time);
            }
            Call::RequestParry { who, bt, ftp } => {
                let fi = self.fi(who);
                self.request_parry(fi, *bt, *ftp);
            }
            Call::PresetAttack { who, mv, angle } => {
                let fi = self.fi(who);
                self.request_preset_attack(fi, *mv, *angle);
            }
            Call::AttackNow { who, mv, angle } => {
                let fi = self.fi(who);
                self.request_attack(fi, *mv, *angle);
            }
            Call::SetHasHitIncludingCosmeticHit { who } => {
                let fi = self.fi(who);
                if let Some(id) = self.fighters[fi].motion {
                    if let Some(a) = self.mm(fi, id).attack_mut() {
                        a.set_has_hit_including_cosmetic_hit();
                    }
                }
            }
            Call::EquipRight { who, weapon } => {
                let fi = self.fi(who);
                // The serialized hook keeps its spelling, but explicitly creates a new actor.
                self.create_and_equip_right(fi, weapon).expect("EquipRight creation");
            }
        }
    }

    /// CombatState._contact: only an attack in Release processes a scripted contact
    fn contact(&mut self, a: usize, b: usize, bone: &str) {
        if self.attack_stage(a) != Some(super::enums::stage::RELEASE) {
            let s = format!("{} contact ignored (not in Release)", self.fighters[a].name);
            self.trace_event(&s);
            return;
        }
        self.process_hit(a, b, bone);
    }

    /// CombatState.trace_event: "%.3f %s"
    pub fn trace_event(&mut self, s: &str) {
        self.events.push(format!("{:.3} {}", self.now, s));
    }

    /// CombatState.emit_event: ev["t"] = now, appended to `hits`
    pub fn emit_event(&mut self, ev: Value) {
        let mut m = match ev {
            Value::Object(m) => m,
            _ => Map::new(),
        };
        m.insert("t".into(), json!(self.now));
        self.hits.push(m);
    }

    /// CombatState.trace_motion
    pub(crate) fn trace_motion(&mut self, fi: usize) {
        use super::enums::{ATTACK_TYPE_NAMES, MOVE_NAMES};
        let m = self.cur_m(fi).unwrap();
        let mut extra = String::new();
        if let Some(a) = m.attack() {
            extra = format!(
                " {} {} windup_end={:.4} release_end={:.4} end={:.4}",
                MOVE_NAMES[a.mv as usize], ATTACK_TYPE_NAMES[a.ty as usize], a.windup_end, a.release_end, m.end_time
            );
        } else if m.feinted().is_some() || m.parry().is_some() || m.blocked().is_some() || m.is_flinch() {
            extra = format!(" end={:.4}", m.end_time);
        }
        let kind = m.kind();
        let name = self.fighters[fi].name.clone();
        self.trace_event(&format!("{name} -> {kind}{extra}"));
        self.emit_event(json!({"kind": "motion", "who": name, "motion": kind}));
    }

    /// fighter name of a Fighter::id ("" when that pawn has left the world)
    pub fn name_of(&self, id: u32) -> String {
        self.fighters.iter().find(|f| f.id == id).map(|f| f.name.clone()).unwrap_or_default()
    }
    pub fn fighter_by_id(&self, id: u32) -> Option<usize> {
        self.fighters.iter().position(|f| f.id == id)
    }

    /// CombatState.remove_fighter (a respawn between rounds): drops it, its pending inputs, its trace memory entries,
    /// and its id from the other fighters' hit lists and ignore caches.
    pub fn remove_fighter(&mut self, name: &str) {
        let Some(fi) = self.fighter_index(name) else { return };
        self.release_pawn_equipment(fi);
        let f = self.fighters.remove(fi);
        for v in self.schedule.values_mut() {
            v.retain(|ev| !input_mentions(ev, name));
        }
        self.attack_traces_memory.retain(|e| e.owner != f.id);
        for oi in 0..self.fighters.len() {
            for m in [self.fighters[oi].motion, self.fighters[oi].last_attack_motion].into_iter().flatten() {
                if let Some(a) = self.mm(oi, m).attack_mut() {
                    a.hit_actors.retain(|x| *x != f.id);
                }
            }
            self.fighters[oi].tracer.actor_ignore_cache.retain(|x| *x != f.id);
        }
    }
}

fn input_mentions(ev: &Input, name: &str) -> bool {
    match ev {
        Input::Attack { who, .. } | Input::Feint { who } | Input::Parry { who, .. } | Input::ReleaseBlock { who } | Input::SwitchMode { who } | Input::ToggleMode { who } => who == name,
        Input::Contact { who, target, .. } => who == name || target == name,
        Input::Call(_) => false,
    }
}

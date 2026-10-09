//! `Sim`: one deterministic fixed-step world = exe-exact combat (mordhau-core World) + per-fighter character movement
//! (mh-character ExeMovement against a host collision `World`) + weapon traces against the posed bodies (trace.rs) +
//! hooks for the game mode and bots (mh-mode drives them through `Sim::combat` / `on_events`).
//!
//! Frame order (the exe's, for locally controlled authority pawns; mh-character exe.rs "Frame order" and
//! mordhau-core combat/world.rs):
//!   0. UWorld::Tick advances TimeSeconds (both clocks: combat World::step_pre and ExeMovement::frame, f32)
//!   1. player input (APlayerController): combat requests (RequestAttack / BlockPressed / ...) and movement axes
//!   2. actor tick, TG_PrePhysics: AMordhauCharacter::LODTick rva=0x154c390 (motion Tick, buffered parry, stamina)
//!   3. UCharacterMovementComponent tick (ExeMovement::frame) with the motion's GetMovementRestriction
//!   4. the pose is evaluated (host animation, or the reference pose) and fed to the trace geometry
//!   5. TG_PostPhysics: ULateTickComponent -> PrepareForTracing + ExecuteAttackTracingAndLogic (combat step_post)
//!   6. bots / game mode (host hook)
//! UNCONFIRMED: the controller's input processing before the pawn's actor tick (only the movement component's
//! prerequisite on the controller is engine-guaranteed); pose evaluation between movement and the late tick (the
//! skeletal mesh component ticks after its owner's movement, AAdvancedCharacter::BeginPlay rva=0x1459110 makes the
//! LateTickComponent wait for the Mesh).

use crate::trace::{Geometry, Posed, SimTrace};
use mh_character::{CharacterRecords, ExeInput, ExeMovement, Mode};
use mordhau_core::combat::world::Input;
use mordhau_core::combat::World;
use mordhau_core::data::Spec;
use mordhau_core::ue::{FQuat, FTransform, FVector};
use serde_json::{json, Map, Value};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

/// A fighter to spawn (UE cm / degrees)
#[derive(Clone, Debug, Default)]
pub struct FighterDesc {
    pub name: String,
    pub weapon: String,
    pub left: String,
    pub team: i64,
    pub location: FVector,
    pub yaw: f32,
}

/// One frame of one fighter's input
#[derive(Clone, Debug, Default)]
pub struct SimInput {
    /// MoveForward / MoveRight axes in [-1, 1]
    pub fwd: f32,
    pub right: f32,
    pub jump: bool,
    pub sprint: bool,
    pub crouch: bool,
    /// control rotation (degrees): yaw turns the actor, look_up = AAdvancedCharacter LookUpValue
    pub yaw: Option<f32>,
    pub look_up: Option<f64>,
    /// the owning client already applied AAdvancedCharacter::Turn rva=0x14a8a90 / LookUp rva=0x1489930 (their rate-cap
    /// clamp and the LODTick rva=0x14887b0 refill run on the client, at its frame rate; the server takes the control
    /// rotation from ServerMove): take `yaw` / `look_up` as the control rotation without cap_turn / cap_look_up.
    /// Bots leave it false (an AI controller's rotation goes through Turn on the server).
    pub turn_applied_by_client: bool,
    /// (EAttackMove, angle degrees)
    pub attack: Option<(i64, f64)>,
    pub feint: bool,
    /// EBlockType: block pressed
    pub parry: Option<i64>,
    /// Host-fed GetAnglingVector().X after the player's current input flush. None for AI/script callers.
    pub controller_angling_x: Option<f32>,
    pub release_block: bool,
    pub switch_mode: bool,
    pub toggle_mode: bool,
    /// the Fire action: Some(true) = FirePressed, Some(false) = FireReleased (ranged weapons; rangedmotion.rs)
    pub fire: Option<bool>,
}

/// Where a fighter's component-space pose comes from each frame
#[derive(Clone, Debug)]
pub enum PoseSource {
    /// the skeleton's reference pose
    Reference,
    /// bones set by the host (animation), component space, UE (`Sim::set_pose`)
    Host(Vec<FTransform>),
    /// the ported animation graph (animgraph.rs: montages from the motions, slots, LayeredBoneBlend_1), the default
    /// once `Sim::enable_anim` has run
    Graph,
}

/// fp-anim r1: AB_MordhauCharacterAnimation CDO AnimRateFactor1PMaxSprint (extract/json AB_MordhauCharacterAnimation.json
/// Default__: 0.15; the UMordhauAnimInstance ctor's 0.25, decomp 7050, is overridden)
pub const ANIM_RATE_FACTOR_1P_MAX_SPRINT: f64 = 0.15;


pub struct Sim {
    pub combat: World,
    pub dt: f32,
    pub movers: Vec<ExeMovement>,
    pub yaw: Vec<f32>,
    pub poses: Vec<PoseSource>,
    pub geo: Rc<Geometry>,
    pub posed: Rc<RefCell<HashMap<String, Posed>>>,
    pub sampled_traces: Rc<RefCell<Vec<crate::trace::SampledTrace>>>,
    world_static_queries: Rc<RefCell<Option<crate::trace::WorldStaticQuery>>>,
    pub ragdolls: Option<crate::ragdoll::Ragdolls>,
    pub records: CharacterRecords,
    pub collision: Box<dyn mh_character::World>,
    /// bots (mh-mode AMordhauAIController + behavior tree) and their body -> fighter map; None = no bots
    pub bots: Option<mh_mode::ai::Bots>,
    pub bot_fighter: Vec<usize>,
    /// the bots' navmesh / collision queries (mh-nav NavMesh; rust-mode-ai r4): with it the bots' move requests follow
    /// navmesh paths (bot_follow) and their NavigationRaycast / GetRandomReachablePointInRadius are answered
    pub nav: Option<Box<dyn mh_mode::ai::NavQueries>>,
    /// per bot body: path following of its move request, and the point it steers at this frame (None: straight)
    pub bot_follow: Vec<mh_mode::ai::PathFollower>,
    pub bot_steer: Vec<Option<FVector>>,
    /// per bot body: this frame's LODTick turn input (yaw, pitch degrees; mh-mode BotController::lod_turn, rva=0x14fe7c0)
    pub bot_turn: Vec<Option<(f32, f32)>>,
    /// the loudness of the noise-making character sounds (noise.rs; the host may fill it from the paks with
    /// NoiseLoudness::read, else the USoundCue ctor default)
    pub noise_loudness: crate::noise::NoiseLoudness,
    noise0: usize,
    /// the animation assets + per-fighter animation state (None = no graph: reference / host poses only)
    pub anim: Option<Rc<crate::animgraph::AnimAssets>>,
    pub fanim: Vec<crate::animgraph::FighterAnim>,
    /// fp-anim r2: the weapon mode the equipment-asset reads take (Sim::read_equipment_anims), None = the fighter's
    alt_override: std::cell::Cell<Option<bool>>,
    /// the game mode (mode.rs; None = no mode: fighters are added by the host)
    pub mode: Option<crate::mode::SimMode>,
    /// running climbs per fighter (climb.rs)
    pub climbs: Vec<Option<crate::climb::ClimbRun>>,
    /// projectiles in flight (ranged.rs)
    pub projectiles: Vec<crate::ranged::SimProjectile>,
    /// projectile classes by path (ranged.rs projectile_class; filled by equip_ranged)
    pub projectile_classes: HashMap<String, (mh_character::projectile::ProjectileCfg, mordhau_core::combat::ranged::ProjectileDamage)>,
    /// horses (horse.rs)
    pub horses: Vec<crate::horse::SimHorse>,
    /// AMordhauCharacter LastKnockback per fighter (horse.rs DoKnockback)
    pub last_knockback: Vec<f64>,
    /// per-fighter edge state of the presentation events (events.rs)
    pub ev_state: Vec<crate::events::EventState>,
    /// the anim instance runs as on a dedicated server (IsDedicatedServer 1, AnimLOD0/1 0: lower.rs); the Sim is the
    /// authority. Set before enable_anim / add_fighter.
    pub dedicated_server: bool,
    /// per fighter: posed as on a dedicated server (the default `dedicated_server`) or as on a standalone / listen-server
    /// game (fidelity-audit r6: offline play is a standalone client, every character IsDedicatedServer 0 there,
    /// UMordhauAnimInstance::NativeUpdateAnimation rva=0x1501930 decomp 500-506)
    pub server_pose: Vec<bool>,
    /// rust-net r7: fighters whose movement a network host drives (a remote player's ServerMoves on the server, mh-net
    /// sim_server.rs): the Sim reads their mover but does not move it
    pub net_moved: std::collections::HashSet<String>,
    /// the host's EPhysicalSurface lookup for a collision hit (component / body index, point), e.g.
    /// mh_level::collision::CollisionWorld::surface_at (FBodyInstance::GetSimplePhysicalMaterial rva=0x32ec370 /
    /// the landscape's cooked physical materials); None = surface 0 (SurfaceType_Default)
    pub surface_of: Option<Box<dyn Fn(u32, FVector) -> u8>>,
    hits0: usize,
    /// the next combat.hits entry the procedural flinch reads (flinch.rs, fidelity-audit r8)
    flinch0: usize,
    log0: usize,
}

impl Sim {
    /// `spec`: exe f32 records (SpecBuilder / RecordsJsonExe); `geo`: physics / skeleton / weapons (load.rs)
    pub fn new(spec: Rc<Spec>, geo: Rc<Geometry>, records: CharacterRecords, collision: Box<dyn mh_character::World>, dt: f32) -> Sim {
        let mut combat = World::new(spec, dt as f64);
        let posed = Rc::new(RefCell::new(HashMap::new()));
        let sampled_traces = Rc::new(RefCell::new(Vec::new()));
        let world_static_queries = Rc::new(RefCell::new(None));
        combat.set_trace_host(Some(Rc::new(SimTrace { geo: geo.clone(), posed: posed.clone(), sampled: sampled_traces.clone(), world_static: world_static_queries.clone() })));
        Sim { combat, sampled_traces, world_static_queries, ragdolls: None, dt, movers: Vec::new(), yaw: Vec::new(), poses: Vec::new(), geo, posed, records, collision, bots: None, bot_fighter: Vec::new(), nav: None, bot_follow: Vec::new(), bot_steer: Vec::new(), bot_turn: Vec::new(), noise_loudness: Default::default(), noise0: 0, anim: None, fanim: Vec::new(), alt_override: Default::default(), mode: None, climbs: Vec::new(), projectiles: Vec::new(), projectile_classes: HashMap::new(), horses: Vec::new(), last_knockback: Vec::new(), dedicated_server: true, server_pose: Vec::new(), net_moved: Default::default(), surface_of: None, ev_state: Vec::new(), hits0: 0, flinch0: 0, log0: 0 }
    }

    /// Register the original complex WorldStatic provider for this newly attached level.
    /// The same callback feeds gameplay contact and the native 50cm presentation retrace.
    pub fn set_world_static_query(&mut self, query: Option<crate::trace::WorldStaticQuery>) {
        *self.world_static_queries.borrow_mut() = query;
    }

    pub fn add_fighter(&mut self, d: &FighterDesc) -> usize {
        let fi = self.combat.add_fighter(&d.name, &d.weapon, &d.left);
        self.combat.fighters[fi].team = d.team;
        let mut m = ExeMovement::new(&self.records, d.location);
        m.world_time = self.combat.now as f32;
        self.movers.push(m);
        self.yaw.push(d.yaw);
        self.server_pose.push(self.dedicated_server);
        self.poses.push(if self.anim.is_some() { PoseSource::Graph } else { PoseSource::Reference });
        self.fanim.push(self.new_fanim(fi));
        self.refresh_pose(fi);
        fi
    }

    /// first-person r1 (combat test level): respawn fighter `fi` as a new character at `location` / `yaw`
    /// (World::respawn_fighter); the movement component keeps its equipment / armor setup and is teleported (velocity 0,
    /// walking, a floor check next tick), the animation instance starts fresh with the same perspective
    pub fn respawn(&mut self, fi: usize, location: FVector, yaw: f32) {
        if let Some(r) = &mut self.ragdolls { r.remove(fi); }
        self.release_horse_pawn(fi, false);
        // This host slot retains its camera ownership while its core pawn is replaced.
        let target = self.combat.fighters[fi].is_view_target;
        let debug_override = self.fanim[fi].view_target_debug_override;
        self.combat.respawn_fighter(fi);
        self.set_view_target(fi, target, debug_override);
        let m = &mut self.movers[fi];
        m.location = location;
        m.velocity = FVector::ZERO;
        m.acceleration = FVector::ZERO;
        m.pending_impulse = FVector::ZERO;
        m.pending_force = FVector::ZERO;
        m.just_teleported = true;
        m.force_next_floor_check = true;
        self.yaw[fi] = yaw;
        self.reset_fanim(fi);
        self.refresh_pose(fi);
    }

    /// rust-character r9: a fighter's held equipment and worn armor as its movement component reads them
    /// (UMordhauMovementComponent::UpdateEquipmentSpeedAndAcceleration rva=0x14dcd30, then its tail call
    /// UpdateArmorSpeedAndAcceleration rva=0x14dc0c0): the weapon / off-hand speed bonuses and overrides and the
    /// armor pieces' SpeedFactor / AccelerationFactor (mh_character::equipment). Hosts call it after spawning a fighter
    /// and whenever its equipment or wearables change.
    pub fn set_movement_gear(
        &mut self,
        fi: usize,
        right: Option<&mh_character::equipment::EquipmentMovement>,
        left: Option<&mh_character::equipment::EquipmentMovement>,
        armor: [Option<mh_character::equipment::ArmorWearable>; 3],
        rules: &mh_character::equipment::ArmorRules,
    ) {
        let m = &mut self.movers[fi];
        m.update_equipment_speed_and_acceleration(left, right, &[]);
        m.update_armor_speed_and_acceleration(armor[0].as_ref(), armor[1].as_ref(), armor[2].as_ref(), rules);
        // the same three slots fill the damage tiers' WearableProtectionCoverageMap (BuildCharacter rva=0x15319e0)
        let ac = |i: usize| armor[i].as_ref().map(|a| a.armor_class);
        self.combat.fighters[fi].wearable_coverage = mordhau_core::combat::damage::wearable_coverage(ac(0), ac(1), ac(2));
    }

    /// the left-hand equipment's actor transform (a shield on the LeftWeapon socket; fidelity-audit r5)
    pub fn left_world(&self, fi: usize) -> Option<FTransform> {
        let f = &self.combat.fighters[fi];
        if f.left_hand_path.is_empty() {
            return None;
        }
        let g = self.geo.weapons.get(&f.left_hand_path)?;
        let posed = self.posed.borrow();
        let p = posed.get(&f.name)?;
        self.geo.weapon_world(p, g)
    }

    /// JumpAnimation / FallingAnimation / LandAnimation of UpdateEquipmentData rva=0x151c4d0 (decomp 6612-6718): the main
    /// equipment's field (the 1P one in first person), except that a main shield with a right-hand equipment takes that
    /// equipment's *AnimationShield(1P) when set. UNCONFIRMED: the horse and unarmed branches are not ported
    pub fn air_anims(&self, fi: usize, first_person: bool) -> crate::air::AirAnims {
        let Some(a) = &self.anim else { return Default::default() };
        let f = &self.combat.fighters[fi];
        let sec = self.second_prefix(fi);
        let right = f.weapon_equip.as_ref().map(|e| e.path.clone()).unwrap_or_default();
        let shield = !f.left_hand_path.is_empty();
        let main = if shield { f.left_hand_path.clone() } else { right.clone() };
        let pick = |base: &str| -> String {
            if shield && !right.is_empty() {
                let p = a.equipment_obj(&right, &format!("{base}Shield{}", if first_person { "1P" } else { "" }));
                if !p.is_empty() {
                    return p;
                }
            }
            // fp-anim r1: the Second* field while the weapon is in its alternate mode (SwitchMode_Implementation swap)
            let pre = if shield { "" } else { sec };
            a.equipment_obj(&main, &format!("{pre}{base}{}", if first_person { "1P" } else { "" }))
        };
        crate::air::AirAnims { jump: pick("JumpAnimation"), falling: pick("FallingAnimation"), land: pick("LandAnimation") }
    }

    /// UpperAdditiveA of UpdateEquipmentData rva=0x151c4d0 (decomp 6626-6662, 6731-6744): the main equipment's
    /// UpperAdditive(1P), else with both hands full the right-hand equipment's ShieldUpperAdditive(1P)
    pub fn upper_additive_asset(&self, fi: usize, first_person: bool) -> String {
        let Some(a) = &self.anim else { return String::new() };
        let f = &self.combat.fighters[fi];
        let right = f.weapon_equip.as_ref().map(|e| e.path.clone()).unwrap_or_default();
        let main = if f.left_hand_path.is_empty() { right.clone() } else { f.left_hand_path.clone() };
        let sec = if f.left_hand_path.is_empty() { self.second_prefix(fi) } else { "" };
        let mut p = a.equipment_obj(&main, &format!("{sec}{}", if first_person { "UpperAdditive1P" } else { "UpperAdditive" }));
        if p.is_empty() && !f.left_hand_path.is_empty() && !right.is_empty() {
            p = a.equipment_obj(&right, if first_person { "ShieldUpperAdditive1P" } else { "ShieldUpperAdditive" });
        }
        p
    }

    /// UMordhauAnimInstance::UpdateEquipmentData rva=0x151c4d0 (decomp UMordhauAnimInstance.cpp 6546-6790): MainEquipment
    /// = LeftHandEquipment when present, else RightHandEquipment; UpperBlendSpaceA = MainEquipment's UpperBlendSpace(1P),
    /// and when that is null and both hands hold equipment, the right-hand equipment's ShieldUpperBlendSpace(1P); the
    /// lower animation likewise MainEquipment's LowerAnimation, else the right-hand equipment's ShieldLowerAnimation
    /// (fidelity-audit r5; the horse / alternate-mode / unarmed branches are not modelled here)
    pub fn upper_lower_assets(&self, fi: usize, first_person: bool) -> (String, String) {
        let Some(a) = &self.anim else { return (String::new(), String::new()) };
        let f = &self.combat.fighters[fi];
        let right = f.weapon_equip.as_ref().map(|e| e.path.clone()).unwrap_or_default();
        let main = if f.left_hand_path.is_empty() { right.clone() } else { f.left_hand_path.clone() };
        // fp-anim r1: SwitchMode_Implementation swaps UpperBlendSpace(1P) / LowerAnimation with the Second* fields while
        // bIsUsingAlternateMode (decomp AMordhauEquipment.cpp 3173-3200), so the anim instance reads the Second* ones
        let sec = if f.left_hand_path.is_empty() { self.second_prefix(fi) } else { "" };
        let mut upper = if sec.is_empty() { a.upper_blend_space_for(&main, first_person) } else { a.equipment_obj(&main, if first_person { "SecondUpperBlendSpace1P" } else { "SecondUpperBlendSpace" }) };
        let mut lower = a.equipment_obj(&main, &format!("{sec}LowerAnimation"));
        if !f.left_hand_path.is_empty() && !right.is_empty() {
            if upper.is_empty() {
                upper = a.equipment_obj(&right, if first_person { "ShieldUpperBlendSpace1P" } else { "ShieldUpperBlendSpace" });
            }
            if lower.is_empty() {
                lower = a.equipment_obj(&right, "ShieldLowerAnimation");
            }
        }
        (upper, lower)
    }

    /// fp-anim r4: the current attack's (WindupEnd, ReleaseJumpBlockTime) for the movement's jump gate
    /// (AMordhauCharacter::CanJumpInternal_Implementation rva=0x1532630, mh_character ExeMovement::attack_jump_block):
    /// ReleaseJumpBlockTime (+0xbb8) from the motion Blueprint's class defaults (e.g. BP_StrikeMotion 0.3), else the
    /// UAttackMotion ctor's 0.25 (decomp UAttackMotion.cpp 3554)
    fn attack_jump_block(&self, fi: usize) -> Option<(f32, f32)> {
        let m = self.combat.cur_m(fi)?;
        let at = m.attack()?;
        let bt = self
            .anim
            .as_ref()
            .filter(|_| !m.bp.is_empty())
            .and_then(|a| a.cdo(&m.bp).get("ReleaseJumpBlockTime").and_then(|v| v.as_f64()))
            .unwrap_or(0.25);
        Some((at.windup_end as f32, bt as f32))
    }

    /// "Second" while fighter `fi`'s weapon is in its alternate mode (mordhau-core Fighter.alternate_mode), else ""
    fn second_prefix(&self, fi: usize) -> &'static str {
        // the anim instance's view of the mode (read_equipment_anims / the fighter's swapped state), else core's flag
        let alt = self.alt_override.get().or_else(|| self.fanim.get(fi).filter(|f| f.alt_init).map(|f| f.alt_seen)).unwrap_or(self.combat.fighters[fi].alternate_mode);
        if alt { "Second" } else { "" }
    }

    fn new_fanim(&self, fi: usize) -> crate::animgraph::FighterAnim {
        match &self.anim {
            // the upper pose: the weapon's LowerAnimation loop (AMordhauEquipment +0x840; FighterAnim's tree-less path)
            Some(a) => {
                let eq = self.combat.fighters[fi].weapon_equip.as_ref();
                let (shield_upper, shield_lower) = self.upper_lower_assets(fi, false);
                let has_left = !self.combat.fighters[fi].left_hand_path.is_empty();
                let idle = if has_left && !shield_lower.is_empty() { shield_lower } else { eq.map(|e| e.lower_animation.clone()).unwrap_or_default() };
                let mut fa = crate::animgraph::FighterAnim::new(&self.geo.skeleton, a, &idle);
                if let Some(e) = eq {
                    fa.cosmetic = a.weapon_cosmetic(&e.path);
                    fa.shoulder_1p = a.shoulder_offsets_1p(&e.path);
                    // the held weapon relative to RightWeapon: ComputeGrippedTransform rva=0x14b70f0 on an identity socket
                    if let Some(g) = self.geo.weapons.get(&e.path) {
                        let id = FTransform::new(mordhau_core::ue::FQuat::IDENTITY, mordhau_core::ue::FVector::ZERO);
                        let rel = crate::pose::gripped(&id, g.right_hand_equip_offset, g.rotation_offset, self.geo.grip_pitch_right, g.grip_location_local);
                        // grip r2: the alternate mode's grip (Second* fields) for its offhand target
                        let rel_alt = if g.grip_modes.is_some() { crate::grip::gripped_mode(&self.geo, &id, g, true, false) } else { rel };
                        fa.offhand_weapon = Some((a.offhand_weapon(&e.path, false, rel), a.offhand_weapon(&e.path, true, rel_alt)));
                    }
                    // AMordhauEquipment UpperBlendSpace (+0x820): the UpperBody machine's Idle BlendSpacePlayer_29
                    fa.upper_bs = a.blend_space(&if has_left { shield_upper.clone() } else { a.upper_blend_space(&e.path) });
                    fa.upper_additive = self.upper_additive_asset(fi, false);
                    fa.air.anims = self.air_anims(fi, false);
                }
                // LowerBodyAnimationA = the main equipment's LowerAnimation (lower.rs)
                let mut lb = crate::lower::LowerBody::new(a, &idle);
                lb.dedicated_server = self.dedicated_server;
                fa.lower = Some(lb);
                fa
            }
            None => Default::default(),
        }
    }

    /// UMordhauAnimInstance::OnTookDamage rva=0x150f7c0 for this frame's melee "hit" events (flinch.rs): the victim's
    /// pose and mesh transform, the hit point (the event's "impact"), the attacker weapon's LastObservedTraceDirection
    /// (PrepareForTracing rva=0x16378e0: GetSafeNormal((CurStart - PrevStart) + (CurEnd - PrevEnd))) and the move
    fn flinch_on_hits(&mut self) {
        let n = self.combat.hits.len();
        let from = self.flinch0.min(n);
        self.flinch0 = n;
        if self.anim.is_none() {
            return;
        }
        let now = self.combat.now;
        for k in from..n {
            let e = self.combat.hits[k].clone();
            if e.get("kind").and_then(|x| x.as_str()) != Some("hit") {
                continue;
            }
            let Some(imp) = e.get("impact").and_then(|x| x.as_array()).filter(|a| a.len() == 3) else { continue };
            let impact = FVector::new(imp[0].as_f64().unwrap_or(0.0) as f32, imp[1].as_f64().unwrap_or(0.0) as f32, imp[2].as_f64().unwrap_or(0.0) as f32);
            let name = |k: &str| e.get(k).and_then(|x| x.as_str()).and_then(|s| self.combat.fighter_index(s));
            let Some(v) = name("victim") else { continue };
            // OnTookDamage runs only for a character not in first person (AMordhauCharacter::OnTookDamage_Implementation
            // rva=0x155be10 decomp 5184-5187, UFlinchMotion::OnBegin decomp 126-131); UNCONFIRMED: NetDamage PackedFlags
            // bit 1 (cosmetic flinch) taken as set for every melee hit
            if self.fanim.get(v).map(|f| f.first_person).unwrap_or(true) {
                continue;
            }
            let mv = e.get("move").and_then(|x| x.as_i64()).unwrap_or(-1);
            let posed = self.posed.borrow();
            let Some(pv) = posed.get(&self.combat.fighters[v].name) else { continue };
            let trace_dir = if e.get("ranged").is_some() {
                None
            } else {
                e.get("attacker").and_then(|x| x.as_str()).and_then(|a| posed.get(a)).map(|pa| {
                    let t = &pa.tracer;
                    (t.cur_start - t.prev_start) + (t.cur_end - t.prev_end)
                })
            };
            let mesh = self.geo.mesh_xf_adj(pv.mesh_z_adjust).then(&pv.actor);
            let cs = pv.bones.clone();
            drop(posed);
            if let Some(fa) = self.fanim.get_mut(v) {
                fa.flinch.on_took_damage(&self.geo.skeleton, &cs, &mesh, impact, trace_dir, mv, now);
            }
        }
    }

    /// fighter `fi` posed as on a dedicated server (true) or as a standalone / listen-server client (false): IsDedicatedServer
    /// and AnimLOD0 / AnimLOD1 of NativeUpdateAnimation rva=0x1501930 (decomp 500-506; a client's AnimLOD1 is 1 for a
    /// character rendered at LOD 0: UNCONFIRMED LOD thresholds, every fighter taken as LOD 0) (fidelity-audit r6)
    pub fn set_server_pose(&mut self, fi: usize, server: bool) {
        if let Some(s) = self.server_pose.get_mut(fi) {
            *s = server;
        }
    }

    /// AMordhauCharacter::bIsFirstPerson for fighter `fi` (the local view target in CameraStyle 1; CameraStyleChanged
    /// rva=0x1532190): its anim instance takes the 1P assets (UMordhauAnimInstance::UpdateEquipmentData rva=0x151c4d0
    /// UpperBlendSpace1P; the motions' FPerspective* FirstPerson sides, FPerspectiveAnimMontage::Get rva=0x165c400).
    /// The traces run on this pose: in standalone / listen-server play the local player is the authority and the exe
    /// traces its own (1P) pose (UAttackMotion::OnLateTick_Implementation rva=0x1631f00 -> ServerSuggestHitDetection,
    /// rust-combat r7 reading); a dedicated server would trace its 3P instance instead (fidelity-audit r3).
    /// EVD_CAM_010: the first-person camera's CameraCollisionLocationOffset (UMordhauCameraComponent +?, world, UE cm)
    /// from its last UpdateFirstPersonCamera (the runtime's camera.rs), read by the next anim update (decomp
    /// UMordhauAnimInstance.cpp 1482-1504)
    pub fn set_camera_collision_offset(&mut self, fi: usize, world: FVector) {
        if let Some(fa) = self.fanim.get_mut(fi) {
            fa.camera_collision_location_offset = world;
        }
    }

    pub fn set_first_person(&mut self, fi: usize, first_person: bool) {
        let Some(fa) = self.fanim.get_mut(fi) else { return };
        if fa.first_person == first_person {
            return;
        }
        fa.first_person = first_person;
        if self.anim.is_none() { return; }
        let (p, _) = self.upper_lower_assets(fi, first_person);
        self.fanim[fi].upper_bs = self.anim.as_ref().and_then(|a| a.blend_space(&p));
        self.fanim[fi].upper_additive = self.upper_additive_asset(fi, first_person);
        self.fanim[fi].air.anims = self.air_anims(fi, first_person);
    }

    /// AAdvancedCharacter::IsViewTarget (0x8e1fb0): local camera ownership, also true in local third person.
    /// `debug_override` identifies the host's synthetic fly1p target; it does not select different assets.
    pub fn set_view_target(&mut self, fi: usize, is_view_target: bool, debug_override: bool) {
        if let Some(f) = self.combat.fighters.get_mut(fi) { f.is_view_target = is_view_target; }
        if let Some(fa) = self.fanim.get_mut(fi) {
            fa.is_view_target = is_view_target;
            fa.view_target_debug_override = is_view_target && debug_override;
        }
    }

    /// A new animation instance keeps the host's camera state before its first pose is evaluated.
    fn reset_fanim(&mut self, fi: usize) {
        let old = &self.fanim[fi];
        let (fp, target, debug_override, ds) = (old.first_person, self.combat.fighters[fi].is_view_target, old.view_target_debug_override, old.dedicated_server);
        self.fanim[fi] = self.new_fanim(fi);
        self.set_first_person(fi, fp);
        self.set_view_target(fi, target, debug_override);
        self.fanim[fi].dedicated_server = ds;
    }

    /// Turn on the ported animation graph: every fighter (present and future) is posed by it, so the traces and
    /// `snapshot()["f"][name]["pose"]` read the same bones
    pub fn enable_anim(&mut self, a: crate::animgraph::AnimAssets) {
        self.anim = Some(Rc::new(a));
        for fi in 0..self.poses.len() {
            self.poses[fi] = PoseSource::Graph;
            self.reset_fanim(fi);
            self.refresh_pose(fi);
        }
    }

    /// The final per-bone LOCAL pose (UE space, parent-relative; bone order = `geo.skeleton.names`) the traces used
    pub fn local_pose(&self, fi: usize) -> Vec<FTransform> {
        match &self.poses[fi] {
            PoseSource::Graph if !self.fanim[fi].local.is_empty() => self.fanim[fi].local.clone(),
            PoseSource::Host(b) => self.geo.skeleton.to_local(b),
            _ => self.geo.skeleton.ref_local.clone(),
        }
    }

    pub fn set_pose(&mut self, fi: usize, bones: Vec<FTransform>) {
        self.poses[fi] = PoseSource::Host(bones);
    }

    fn refresh_pose(&mut self, fi: usize) {
        if self.ragdolls.is_some() && self.combat.fighters[fi].dead { return; }
        let name = self.combat.fighters[fi].name.clone();
        let actor = FTransform::new(FQuat::from_rotator(0.0, self.yaw[fi], 0.0), self.movers[fi].location);
        let bones = match &self.poses[fi] {
            PoseSource::Reference => self.geo.skeleton.ref_pose(),
            PoseSource::Host(b) => b.clone(),
            PoseSource::Graph => match &self.anim {
                Some(a) => {
                    let now = self.combat.now;
                    let mut fa = std::mem::take(&mut self.fanim[fi]);
                    let (ui, mss) = self.upper_bs_input_fp(fi, fa.upper_input, fa.first_person);
                    fa.upper_input = ui;
                    fa.movement_speed_scale = mss;
                    // fp-anim r1: the mode switch (AMordhauEquipment::SwitchMode_Implementation swaps the Upper /
                    // Second blend spaces and plays ModeSwitchAnimation; see `sync_alternate_mode`)
                    self.sync_alternate_mode(fi, &mut fa, now);
                    // grip r2: RightWeaponBoneCosmeticTransform of the current mode and perspective (SwitchMode swaps the
                    // Second* pair in; NativeUpdateAnimation rva=0x1501930 decomp 2919-2933 reads the 1P one in 1P)
                    if let Some(e) = self.combat.fighters[fi].weapon_equip.as_ref() {
                        fa.cosmetic = crate::grip::cosmetic(&a.cdo(&e.path), self.combat.fighters[fi].alternate_mode, fa.first_person);
                    }
                    {
                        let v = self.movers[fi].velocity;
                        let rlb = self.combat.cur_m(fi).and_then(|m| m.attack()).map(|x| x.native == "UKickMotion" && x.stage != 2).unwrap_or(false);
                        fa.turn = crate::procedural::TurnInput { yaw: self.yaw[fi], loc: self.movers[fi].location, vel2: (v.x * v.x + v.y * v.y).sqrt(), right_leg_bending: rlb };
                        fa.dedicated_server = self.server_pose.get(fi).copied().unwrap_or(self.dedicated_server);
                        // bIsAirborne: AMordhauCharacter's IsAirborne vcall = the movement in MOVE_Falling (ExeMovement::is_airborne)
                        fa.airborne = self.movers[fi].is_airborne();
                    }
                    if let Some(l) = fa.lower.as_mut() {
                        // Helper_LBHipsZOverrideAlpha = min(IsDedicatedServer + IsFirstPersonFloat, 1) (UpdateBlueprintHelpers
                        // rva=0x151a2b0 decomp 4578-4582): the hips override also runs for a first-person view target
                        l.dedicated_server = fa.dedicated_server || fa.first_person;
                        let v = self.movers[fi].velocity;
                        l.input = (((v.x * v.x + v.y * v.y) as f64).sqrt(), fa.upper_input.0);
                        // fp-anim r1: TwoWayBlend 684 (IsFirstPersonFloat), BlendSpacePlayer_1's Y = Velocity and rate
                        l.first_person = fa.first_person;
                        // EVD_CAM_021: bIsCrouching = the character's bIsCrouched (ACharacter +0x330 bit 0; decomp 1877-1878)
                        l.crouching = self.movers[fi].is_crouched;
                        // EVD_CAM_022: the lower body's world yaw = the mesh yaw + LowerBodyRotationOffset (last update's)
                        l.lb_yaw = (self.yaw[fi] + fa.proc.lower_rot_offset) as f64;
                        l.anim_lod1 = !fa.dedicated_server;
                        l.velocity_1p = fa.upper_input.1;
                        l.movement_speed_scale = fa.movement_speed_scale;
                        // EVD_MOV_019: DirectionOffset inputs - Spine1's component yaw in the last evaluated pose
                        // (GetSocketTransform(Spine1, RTS_Component).Rotator().Yaw), the montage instances' weights, the override
                        {
                            let posed = self.posed.borrow();
                            if let (Some(p), Some(s1)) = (posed.get(&name), self.geo.skeleton.find("Spine1")) {
                                if let Some(b) = p.bones.get(s1) {
                                    let q = b.rot;
                                    l.spine1_yaw = (2.0 * (q.w * q.z + q.x * q.y)).atan2(1.0 - 2.0 * (q.y * q.y + q.z * q.z)).to_degrees() as f64;
                                }
                            }
                        }
                        let spec = self.combat.spec.clone();
                        l.montage_weight = fa.ma.montages.insts.iter().map(|m| m.weight(&spec, now)).sum();
                        l.additive_override_active = fa.ma.additive.ty != crate::additive::NONE;
                    }
                    fa.mesh_rot = self.geo.mesh_xf_adj(self.movers[fi].half_height_adjust()).then(&actor).rot;
                    // EVD_MOV_003: PreGrounding / UpdateGrounding (grounding.rs) with the last evaluated pose's feet
                    // (GetSocketLocation reads the previous evaluation), then RootTranslationOffset / GroundingWeight
                    // to the 1P ModifyBone_96 / _97 (procedural.rs grounding_1p)
                    {
                        let m = &self.movers[fi];
                        let posed = self.posed.borrow();
                        let feet = posed.get(&name).and_then(|p| {
                            let (r, l) = (self.geo.skeleton.find("RightFoot")?, self.geo.skeleton.find("LeftFoot")?);
                            (p.bones.len() == self.geo.skeleton.names.len()).then(|| [self.geo.bone_world(p, r).loc, self.geo.bone_world(p, l).loc])
                        });
                        if let Some(feet) = feet {
                            // bWantsGrounding (UMordhauAnimInstance.cpp 4256-4266): false while the last attack is a kick
                            // (Move 4) that is the current motion or got blocked
                            let f = &self.combat.fighters[fi];
                            let kick = f.last_attack_motion.and_then(|id| f.motions.get(id.0 as usize).and_then(|m| m.as_ref())).and_then(|m| m.attack().map(|a| a.mv)) == Some(mordhau_core::combat::enums::mv::KICK);
                            let cur_blocked = self.combat.cur_m(fi).is_some_and(|m| m.kind() == "Blocked");
                            let last_is_cur = f.last_attack_motion.is_some() && f.last_attack_motion == f.motion;
                            let wants = !(kick && (cur_blocked || last_is_cur));
                            let half = m.capsule_half_height; // the current (crouched or not) capsule half height = Bounds.BoxExtent.Z
                            let fl = &m.current_floor;
                            let col = &self.collision;
                            let trace = |a: FVector, b: FVector| col.line_trace(a, b).filter(|h| h.blocking_hit).map(|h| (h.impact_point, h.impact_normal));
                            let mesh = self.geo.mesh_xf_adj(m.half_height_adjust()).then(&actor).loc;
                            let inp = crate::grounding::GroundingInput {
                                dt: self.dt,
                                airborne: m.is_airborne(),
                                wants_grounding: wants,
                                crouched: m.is_crouched,
                                mesh_xy: (mesh.x, mesh.y),
                                capsule_bottom: m.location.z - half,
                                floor_normal: (fl.blocking_hit && fl.walkable_floor).then_some(fl.hit.impact_normal),
                                feet,
                                trace: &trace,
                            };
                            fa.grounding.update(&inp);
                        }
                        fa.proc.root_translation_offset = fa.grounding.root_translation_offset;
                        fa.proc.grounding_weight = fa.grounding.weight;
                    }
                    fa.update(&self.combat, fi, now, &self.geo.skeleton, a);
                    let cs = fa.last_cs.clone();
                    self.fanim[fi] = fa;
                    cs
                }
                None => self.geo.skeleton.ref_pose(),
            },
        };
        let mut posed = self.posed.borrow_mut();
        let p = posed.entry(name).or_default();
        p.actor = actor;
        p.mesh_z_adjust = self.movers[fi].half_height_adjust();
        p.bones = bones;
        // grip r2 (grip.rs): the held weapon's mode grip and a running mode switch's mesh placement
        let f = &self.combat.fighters[fi];
        // fp-anim r3: the running UEquipmentModeSwitchMotion's (StartTime, bIsSwitchingToAlt) for the grip's stages
        let msw = self.combat.cur_m(fi).and_then(|m| m.mode_switch().map(|x| (m.start_time, x.b_is_switching_to_alt)));
        p.grip.first_person = self.fanim[fi].first_person;
        crate::grip::update(&self.geo, p, &f.weapon_path, f.alternate_mode, self.combat.now, msw);
    }

    /// UMordhauAnimInstance::NativeUpdateAnimation rva=0x1501930's third-person upper blend space input (decomp
    /// UMordhauAnimInstance.cpp 2380-2530): Direction = UAnimInstance::CalculateDirection(Velocity, the mesh rotation
    /// + 90 yaw = the actor rotation) when the 2D speed is above 1 (else kept); Helper_UBVelocity = speed <= MaxWalkSpeed
    /// * MovementSpeedScale (W) ? clamp(2 speed / W, 0, 1) * 60 : clamp((speed - W) / (W * PartialSprintModifier - W),
    /// 0, 1) * 30 + 60, W = MaxWalkSpeed x GetSpeedFactor(0) (EVD_MOV_004). UNCONFIRMED: the extra yaw term fVar96 (DirectionOffset /
    /// LowerBodyRotationOffset) taken as 0, crouching (MaxWalkSpeedCrouched) not modelled.
    /// fp-anim r1: NativeUpdateAnimation rva=0x1501930 decomp 2443-2522: for a first-person character (bIsFirstPerson
    /// and Role >= ROLE_AutonomousProxy; the local player in standalone) Velocity = Helper_UBVelocity is quantized:
    /// 2D speed <= 1 -> 0; sprinting -> 90, with MovementSpeedScale *= 1 + clamp(SprintTime /
    /// SprintTimeToReachMaxSprint, 0, 1) * AnimRateFactor1PMaxSprint (0.15: AB_MordhauCharacterAnimation CDO, over the
    /// ctor's 0.25 at decomp 7050); else 60. Sprinting = MovementModifier > Backpedal (Partial / Sprint / Rush / Chase /
    /// Super), or GetMovementRestriction < 2 with bWantsSprint / bWantsSupersprint; then cleared by the movement vcall
    /// +0x558 (UNCONFIRMED: taken as IsCrouching) or a mode other than Walking / NavWalking, and by MovementModifier 1 / 2
    /// (Sideways / Backpedal). Third person keeps the ramp (upper_bs_input). Returns (input, MovementSpeedScale); the
    /// base MovementSpeedScale is GetSpeedFactor(0) (decomp 2313 / 2445, EVD_MOV_004).
    fn upper_bs_input_fp(&self, fi: usize, prev: (f64, f64), first_person: bool) -> ((f64, f64), f64) {
        let tp = self.upper_bs_input(fi, prev);
        if !first_person {
            return (tp, 1.0);
        }
        use mh_character::exe::{Mode, Sprint};
        let m = &self.movers[fi];
        let v = m.velocity;
        let speed = (v.x * v.x + v.y * v.y).sqrt();
        let mut sprint = m.sprint_state > Sprint::Backpedal;
        if m.get_movement_restriction() < 2 && (m.wants_sprint || m.wants_supersprint) {
            sprint = true;
        }
        if m.is_crouched || !matches!(m.mode, Mode::Walking | Mode::NavWalking) {
            sprint = false;
        }
        if matches!(m.sprint_state, Sprint::Sideways | Sprint::Backpedal) {
            sprint = false;
        }
        // EVD_MOV_004: MovementSpeedScale = UMordhauMovementComponent::GetSpeedFactor(0) (NativeUpdateAnimation decomp
        // 2313 / 2445; was taken as 1); the record reads 0.81 for its loadout
        let mut scale = m.get_speed_factor(0.0) as f64;
        let vel = if speed <= 1.0 {
            0.0
        } else if sprint {
            let t = m.c.sprint_time_to_reach_max_sprint;
            let f = if t.abs() > 1e-8 { (m.sprint_time / t).clamp(0.0, 1.0) } else if m.sprint_time < t { 0.0 } else { 1.0 };
            scale *= f as f64 * ANIM_RATE_FACTOR_1P_MAX_SPRINT + 1.0;
            90.0
        } else {
            60.0
        };
        ((tp.0, vel), scale)
    }

    /// fp-anim r2/r3: the weapon mode switch as the anim instance sees it (state/proofs/fp_anim_items.md item 4).
    /// UEquipmentModeSwitchMotion::OnBegin_Implementation rva=0x165fbb0: SetSpineSpaceAdditiveTarget(zero, 0.25) and
    /// PlayAnim(SwitchingEquipment->ModeSwitchAnimation, 1.0) (the pre-swap field; none -> StopAnim(0.4));
    /// OnTick_Implementation rva=0x1667e00 (decomp 702-705): once TimeSeconds > StartTime + 0.15, FinishSwitch rva=0x165b4d0
    /// -> AMordhauCharacter::SwitchModeAndReAttach, i.e. AMordhauEquipment::SwitchMode_Implementation (decomp
    /// AMordhauEquipment.cpp 3041-3200) swaps UpperBlendSpace(1P) / UpperAdditive(1P) / LowerAnimation with the Second*
    /// fields, and UpdateEquipmentData rva=0x151c4d0 moves the anim instance to them (BlendListByBool_7, 0.5 s). mordhau-core
    /// still flips Fighter.alternate_mode at the request (switch_mode, no UEquipmentModeSwitchMotion: UNCONFIRMED timing on
    /// the combat side); here the flip is taken as the motion's StartTime and the anim assets swap 0.15 s later.
    fn sync_alternate_mode(&self, fi: usize, fa: &mut crate::animgraph::FighterAnim, now: f64) {
        let _ = now;
        fa.switch_hand_grips = None;
        let alt = self.combat.fighters[fi].alternate_mode;
        let Some(a) = self.anim.clone() else { return };
        if !fa.alt_init {
            fa.alt_init = true;
            fa.alt_seen = alt;
            self.read_equipment_anims(fi, fa, alt);
        }
        // fp-anim r3: mordhau-core runs UEquipmentModeSwitchMotion (modeswitch.rs); its OnBegin plays the current
        // ModeSwitchAnimation and sets the spine target, its FinishSwitch (StartTime + 0.15) flips the mode
        if let Some(m) = self.combat.cur_m(fi) {
            if let Some(s) = m.mode_switch() {
                let right = self.combat.fighters[fi].weapon_equip.as_ref().map(|e| e.path.as_str()).unwrap_or("");
                if let Some(g) = self.geo.weapons.get(right) {
                    if let Some(modes) = &g.grip_modes {
                        let target = s.b_is_switching_to_alt;
                        let ro = modes[target as usize].rotation_offset;
                        let identity = FTransform::IDENTITY;
                        fa.switch_hand_grips = Some(crate::procedural::SwitchHandGrips {
                            right: crate::grip::gripped_mode(&self.geo, &identity, g, target, false),
                            left: crate::grip::gripped_mode(&self.geo, &identity, g, target, true),
                            rotation_offset: mordhau_core::ue::FQuat::from_rotator(ro[0], ro[1], ro[2]),
                        });
                    }
                }
            }
            if let Some(s) = m.mode_switch().filter(|_| fa.alt_pending.map(|p| p.1) != Some(m.start_time)) {
                let t0 = m.start_time;
                let source_alt = !s.b_is_switching_to_alt;
                fa.alt_pending = Some((s.b_is_switching_to_alt, t0));
                let right = self.combat.fighters[fi].weapon_equip.as_ref().map(|e| e.path.clone()).unwrap_or_default();
                let key = if source_alt { "SecondModeSwitchAnimation" } else { "ModeSwitchAnimation" };
                let spec = self.combat.spec.clone();
                match a.montage(&a.equipment_obj(&right, key)) {
                    Some(mt) => {
                        fa.ma.montages.play(&spec, mt, t0, 1.0);
                    }
                    None => fa.ma.montages.stop_all(&spec, t0, 0.4),
                }
                fa.ma.spine.set_target(mordhau_core::data::SpineAdd::new(), 0.25, t0);
            }
        }
        if alt != fa.alt_seen {
            fa.alt_seen = alt;
            self.read_equipment_anims(fi, fa, alt);
        }
    }

    /// the anim instance's equipment assets (UpdateEquipmentData) for weapon mode `alt`
    fn read_equipment_anims(&self, fi: usize, fa: &mut crate::animgraph::FighterAnim, alt: bool) {
        let Some(a) = &self.anim else { return };
        self.alt_override.set(Some(alt));
        let (p, lower) = self.upper_lower_assets(fi, fa.first_person);
        fa.upper_bs = a.blend_space(&p);
        fa.upper_additive = self.upper_additive_asset(fi, fa.first_person);
        fa.air.anims = self.air_anims(fi, fa.first_person);
        self.alt_override.set(None);
        // fp-anim r2: LeftShoulderIdleOffset1P of the left-hand equipment (NativeUpdateAnimation decomp 3887-3902) and
        // HandSpringWeightTarget = MainEquipment && !bDisableHandSpringAnimation (UpdateEquipmentData decomp 6572,
        // 6604-6611; MainEquipment = the left-hand equipment, else the right-hand one)
        let f = &self.combat.fighters[fi];
        let right = f.weapon_equip.as_ref().map(|e| e.path.clone()).unwrap_or_default();
        let main = if f.left_hand_path.is_empty() { right } else { f.left_hand_path.clone() };
        fa.left_shoulder_idle_1p = if f.left_hand_path.is_empty() {
            FVector::ZERO
        } else {
            let d = a.cdo(&f.left_hand_path);
            let v = d.get("LeftShoulderIdleOffset1P").cloned().unwrap_or_default();
            let g = |k: &str| v[k].as_f64().unwrap_or(0.0) as f32;
            FVector::new(g("X"), g("Y"), g("Z"))
        };
        fa.proc.hand_spring_target = if main.is_empty() {
            0.0
        } else if a.cdo(&main).get("bDisableHandSpringAnimation").and_then(|v| v.as_bool()).unwrap_or(false) {
            0.0
        } else {
            1.0
        };
        if let Some(l) = fa.lower.as_mut() {
            if !lower.is_empty() {
                l.idle = lower;
            }
        }
    }

    fn upper_bs_input(&self, fi: usize, prev: (f64, f64)) -> (f64, f64) {
        let m = &self.movers[fi];
        let v = m.velocity;
        let speed = (v.x * v.x + v.y * v.y).sqrt();
        let dir = if 1.0 < speed { mh_character::uequat::calculate_direction(v, 0.0, self.yaw[fi], 0.0) as f64 } else { prev.0 };
        // EVD_MOV_004: W = MaxWalkSpeed (MaxWalkSpeedCrouched when crouching) x GetSpeedFactor(0) (decomp 2435-2443)
        let w = (if m.is_crouched { m.c.max_walk_speed_crouched } else { m.c.max_walk_speed }) * m.get_speed_factor(0.0);
        let vel = if speed <= w {
            ((2.0 / w) * speed).clamp(0.0, 1.0) * 60.0
        } else {
            ((speed - w) / (w * m.c.partial_sprint_modifier - w)).clamp(0.0, 1.0) * 30.0 + 60.0
        };
        (dir, vel as f64)
    }

    /// FighterGeom of fighter `fi` from its pose: RootComponent = the capsule centre; CameraLocation1P /
    /// Full CameraRotation1P from UpdateFPCamera rva=0x1571ac0 (geometry::camera_1p_full); the BlockCollider placed by AMordhauCharacter::UpdateBlockCollider rva=0x15700a0 (geometry.rs
    /// update_block_collider: offsets blended by the parry height, under the 1P camera) with its scaled BoxExtent
    pub fn refresh_geom(&mut self, fi: usize) {
        let name = self.combat.fighters[fi].name.clone();
        let posed = self.posed.borrow();
        let Some(p) = posed.get(&name) else { return };
        let look = self.combat.fighters[fi].look_up_value as f32;
        // UpdateFPCamera: the 1P camera from the "Position" bone's rotation and the "Spine1" bone's location
        // (geometry::camera_1p); CharacterMesh0 has no RelativeScale3D in BP_MordhauCharacter, so scale Z = 1
        let sk = &self.geo.skeleton;
        let (cam_loc, cam_rot) = match (sk.find("Position"), sk.find("Spine1")) {
            (Some(pb), Some(sb)) => mordhau_core::combat::geometry::camera_1p_full(self.geo.bone_world(p, pb).rot, self.geo.bone_world(p, sb).loc, 1.0, look),
            _ => (p.actor.loc, (look, self.yaw[fi],0.0)),
        };
        // the ParryWeapon's ParryBoxTransform while a parry is current (UpdateBlockCollider)
        let parrying = self.combat.cur_m(fi).map(|m| m.is_parry()).unwrap_or(false);
        let pb = if parrying {
            self.combat.parry_weapon_equip(fi).and_then(|e| self.geo.weapons.get(&e.path)).and_then(|g| g.parry_box)
        } else {
            None
        };
        let [o, l, h] = &self.geo.block_collider_offsets;
        let (bc, ext) = mordhau_core::combat::geometry::update_block_collider_pb_full(o, l, h, pb.as_ref(), look, cam_loc, cam_rot, self.geo.block_collider_extent);
        let g = mordhau_core::combat::world::FighterGeom { root: p.actor.loc, camera_loc: cam_loc, camera_rot: (cam_rot.0,cam_rot.1), block_collider: bc, block_extent: ext };
        drop(posed);
        self.combat.fighters[fi].geom = Some(g);
    }

    /// One frame of variable length `dt` (seconds), as the exe's client ticks: UWorld::Tick advances TimeSeconds by
    /// the frame's DeltaSeconds and runs the tick groups once per rendered frame: the controller, then the movement
    /// component (UCharacterMovementComponent::TickComponent, which substeps PhysWalking / PhysFalling by
    /// MaxSimulationTimeStep 0.05 and MaxSimulationIterations 8: mh-character exe.rs, ctor values 0x142f6d094), then
    /// AMordhauCharacter::LODTick rva=0x154c390 -> UMotionSystemComponent::OnLODTick rva=0x14cb3e0 (the motions'
    /// Tick / LateTick with the same DeltaTime). The frame length is the engine's (the user's FrameRateLimit cap is the
    /// host's business). Only for exe precision: the reference clock (tick_n * dt) needs a fixed dt.
    pub fn step_dt(&mut self, inputs: &[(usize, SimInput)], dt: f32) {
        let dt = dt.max(0.0);
        self.dt = dt;
        self.combat.dt = dt as f64;
        self.step(inputs);
    }

    /// One fixed step with this frame's inputs (fighter index, input)
    pub fn step(&mut self, inputs: &[(usize, SimInput)]) {
        self.sampled_traces.borrow_mut().clear();
        let previous_velocity: Vec<_> = self.movers.iter().map(|m| m.velocity).collect();
        let n = self.combat.tick_n + 1;
        for (fi, i) in inputs {
            self.combat.fighters[*fi].controller_angling_x = i.controller_angling_x;
            let who = self.combat.fighters[*fi].name.clone();
            // the control rotation through AAdvancedCharacter::Turn rva=0x14a8a90's turn cap (mordhau-core turncap.rs:
            // an attack's TurnCaps rate-limit the turn; uncapped otherwise). The yaw change is taken the short way.
            if i.turn_applied_by_client {
                if let Some(y) = i.yaw {
                    self.yaw[*fi] = y;
                }
                if let Some(l) = i.look_up {
                    self.combat.fighters[*fi].look_up_value = l;
                }
            } else if let Some(y) = i.yaw {
                let cur = self.yaw[*fi];
                let mut d = (y - cur) % 360.0;
                if d > 180.0 {
                    d -= 360.0;
                } else if d < -180.0 {
                    d += 360.0;
                }
                let applied = self.combat.cap_turn(*fi, d as f64) as f32;
                self.yaw[*fi] = if applied == d { y } else { cur + applied };
            }
            if let (false, Some(l)) = (i.turn_applied_by_client, i.look_up) {
                let cur = self.combat.fighters[*fi].look_up_value;
                let applied = self.combat.cap_look_up(*fi, l - cur);
                self.combat.fighters[*fi].look_up_value = if applied == l - cur { l } else { cur + applied };
            }
            let mut push = |ev: Input| self.combat.push(n, ev);
            if i.release_block {
                push(Input::ReleaseBlock { who: who.clone() });
            }
            if let Some(b) = i.parry {
                push(Input::Parry { who: who.clone(), bt: b });
            }
            if i.feint {
                push(Input::Feint { who: who.clone() });
            }
            if let Some((mv, a)) = i.attack {
                push(Input::Attack { who: who.clone(), mv, angle: a });
            }
            if i.switch_mode {
                push(Input::SwitchMode { who: who.clone() });
            }
            if i.toggle_mode {
                push(Input::ToggleMode { who: who.clone() });
            }
            if let Some(f) = i.fire {
                // AMordhauCharacter::FirePressed rva=0x153d940 / FireReleased rva=0x153e890 (input, before the tick)
                self.combat.set_wants_fire(*fi, f);
            }
        }
        // 0-2: clock, combat input, actor ticks
        self.combat.step_pre(&mut |_| {});
        // 3: movement with the current motion's restriction (a mounted rider has no movement of its own: the horse
        // moves with the rider's input and carries it, horse.rs)
        self.horse_inputs(inputs);
        // the capsules other characters block on (pawns.rs): live, not riding (a rider's capsule is PawnOnVehicle)
        let solid: Vec<bool> = (0..self.movers.len()).map(|fi| !self.combat.fighters[fi].dead && self.riding(fi).is_none()).collect();
        for fi in 0..self.movers.len() {
            if self.riding(fi).is_some() {
                continue;
            }
            if self.net_moved.contains(&self.combat.fighters[fi].name) {
                // rust-net r7: moved by its ServerMoves (the host copied the mover in); only the combat side's reads
                let m = &self.movers[fi];
                let f = &mut self.combat.fighters[fi];
                f.airborne = m.mode == Mode::Falling;
                f.velocity = m.velocity;
                f.actor_xf = None;
                continue;
            }
            let inp = inputs.iter().find(|(f, _)| *f == fi).map(|x| x.1.clone()).unwrap_or_default();
            let ei = ExeInput { fwd: inp.fwd, right: inp.right, jump: inp.jump, sprint: inp.sprint, crouch: inp.crouch, yaw: self.yaw[fi] };
            self.climb_gates(fi);
            let mr = self.combat.movement_restriction(fi);
            let dead = self.combat.fighters[fi].dead;
            // the current motion's SpeedFactor / BackpedalSpeedFactor, which OnCharacterLODTick rva=0x14c9ec0 copies
            // into the movement on the actor tick (mh_character ExeMovement::on_character_lod_tick)
            let mf = self.combat.motion_speed_factors(fi).map(|(a, b)| (a as f32, b as f32));
            let jb = self.attack_jump_block(fi);
            let m = &mut self.movers[fi];
            m.current_motion_factors = mf;
            m.attack_jump_block = jb;
            m.motion_restriction = mr;
            m.dead = dead;
            // every other live character's CollisionCylinder where it stands now (the earlier movers of this frame
            // have moved, as the exe's tick order has them)
            let pawns = (0..self.movers.len())
                .filter(|&o| o != fi && solid[o])
                .map(|o| {
                    let om = &self.movers[o];
                    crate::pawns::PawnCapsule { id: crate::pawns::PAWN_COMPONENT_BASE + o as u32, centre: om.location, radius: om.e.capsule_radius, half_height: om.capsule_half_height }
                })
                .collect();
            let world = crate::pawns::PawnWorld { inner: self.collision.as_ref(), pawns };
            let m = &mut self.movers[fi];
            m.frame(&world, self.dt, &ei);
            let f = &mut self.combat.fighters[fi];
            f.airborne = m.mode == Mode::Falling; // ReplicatedCharacterFlags bit 0 (IsAirborne)
            f.velocity = m.velocity;
            f.actor_xf = None;
            self.climb_post(fi);
            self.movement_events(fi);
        }
        // 4: poses, then the UE-space geometry the exe's parry / chamber / clash rules read (FighterGeom)
        for fi in 0..self.movers.len() {
            self.refresh_pose(fi);
            self.refresh_geom(fi);
        }
        // 5: late ticks + traces, then projectiles (ranged.rs) and horses (horse.rs)
        self.combat.step_post();
        if let Some(mut ragdolls) = self.ragdolls.take() {
            ragdolls.tick(self, &previous_velocity);
            self.ragdolls = Some(ragdolls);
            for fi in 0..self.movers.len() { if self.combat.fighters[fi].dead { self.refresh_geom(fi); } }
        }
        self.flinch_on_hits();
        self.tick_projectiles();
        self.tick_horses();
        self.post_events();
        // 6: bots (BotSim's frame: bodies refreshed from the fighters, then every bot's AI frame at the world time)
        self.report_noises();
        self.tick_bots();
        // 7: the game mode: this frame's hits / deaths scored, then its tick; pawns it spawns or destroys take effect
        // from the next frame (fighter indices stay valid for this frame's inputs; UNCONFIRMED tick-group order)
        self.mode_post();
        self.mode_pre();
    }

    /// Bot inputs for the next frame (the host merges them with player inputs): movement toward the bot's
    /// MoveToLocation request and its facing (AMordhauAIController facing modes). UNCONFIRMED: straight-line path
    /// following (no navmesh in the sim; mh-mode's BotHost nav queries stay unanswered), turn at once to the facing yaw.
    pub fn bot_inputs(&self) -> Vec<(usize, SimInput)> {
        let Some(b) = &self.bots else { return Vec::new() };
        let mut out = Vec::new();
        for c in b.bots.iter().flatten() {
            let body = &b.bodies[c.body];
            let fi = self.bot_fighter[c.body];
            let me = self.movers[fi].location;
            let mut inp = SimInput { sprint: body.wants_sprint, crouch: body.wants_crouch, ..Default::default() };
            // the turn: AMordhauAIController::LODTick's AddControllerYawInput (rate-limited by the profile's MaxTurnRate,
            // smoothed through RotationTargetInterpolated; rust-mode-ai r6), computed in tick_bots
            if let Some(Some((dy, _))) = self.bot_turn.get(c.body) {
                inp.yaw = Some(self.yaw[fi] + dy);
            }
            if let Some(m) = c.move_request {
                let fin = m.dest - me;
                // the navmesh path's next corner when a nav is attached (bot_steer), else straight at the destination
                let d = self.bot_steer.get(c.body).copied().flatten().unwrap_or(m.dest) - me;
                let dist = (d.x * d.x + d.y * d.y).sqrt();
                if ((fin.x * fin.x + fin.y * fin.y).sqrt() as f64) > m.acceptance && dist > 0.0 {
                    let yaw = inp.yaw.unwrap_or(self.yaw[fi]).to_radians();
                    let (fx, fy) = (yaw.cos(), yaw.sin());
                    let (dx, dy) = (d.x / dist, d.y / dist);
                    inp.fwd = dx * fx + dy * fy;
                    inp.right = dy * fx - dx * fy;
                }
            }
            out.push((fi, inp));
        }
        out
    }

    /// this frame's character sounds as AI noise events (noise.rs: PlayCharacterSound rva=0x155dac0 ->
    /// UAISense_Hearing::ReportNoiseEvent at the character's root)
    fn report_noises(&mut self) {
        let n = self.combat.hits.len();
        let from = self.noise0.min(n);
        self.noise0 = n;
        let Some(bots) = self.bots.as_mut() else { return };
        let now = self.combat.now;
        for e in &self.combat.hits[from..n] {
            let kinds = crate::noise::NoiseLoudness::kinds_of(e);
            if kinds.is_empty() {
                continue;
            }
            let Some(who) = e.get("who").and_then(|v| v.as_str()) else { continue };
            let Some(fi) = self.combat.fighter_index(who) else { continue };
            let Some(body) = self.bot_fighter.iter().position(|&f| f == fi) else { continue };
            let weapon = self.combat.fighters[fi].weapon_equip.as_ref().map(|w| w.path.clone()).unwrap_or_default();
            for (kind, per_weapon) in kinds {
                if let Some(loud) = self.noise_loudness.loudness(kind, per_weapon, &weapon) {
                    bots.report_noise(body, self.movers[fi].location, loud, 0.0, now);
                }
            }
        }
    }

    fn tick_bots(&mut self) {
        let Some(mut bots) = self.bots.take() else { return };
        for (bi, &fi) in self.bot_fighter.iter().enumerate() {
            let body = &mut bots.bodies[bi];
            mh_mode::ai::combat_host::refresh_body(&self.combat, fi, body);
            let m = &self.movers[fi];
            body.location = m.location;
            body.velocity = m.velocity;
            body.yaw = self.yaw[fi] as f64;
            if let Some(p) = self.posed.borrow().get(&self.combat.fighters[fi].name) {
                body.prev_trace_end = p.tracer.prev_end;
                body.trace_start = p.tracer.cur_start;
                body.trace_end = p.tracer.cur_end;
            }
        }
        let (dt, now) = (self.combat.dt, self.combat.now);
        let mut host = mh_mode::ai::combat_host::CombatHost { world: &mut self.combat, fighter_of: self.bot_fighter.clone() };
        // the sight sense's line of sight against the static world (rust-mode-ai r6: controller.rs sight_stimulus)
        let coll = &*self.collision;
        let blocked = |a: FVector, b: FVector| coll.line_trace(a, b).is_some();
        let mut host = mh_mode::ai::WithSight { inner: &mut host, blocked: &blocked };
        match self.nav.as_mut() {
            Some(nav) => {
                let mut h = mh_mode::ai::WithNav { inner: &mut host, nav: &mut **nav };
                bots.tick(dt, now, &mut h);
            }
            None => bots.tick(dt, now, &mut host),
        }
        // LODTick turning (rust-mode-ai r6): the facing mode's target, Movement = the path following's direction
        let n = self.bot_fighter.len();
        self.bot_turn.resize(n, None);
        {
            let mut host = mh_mode::ai::combat_host::CombatHost { world: &mut self.combat, fighter_of: self.bot_fighter.clone() };
            for i in 0..bots.bots.len() {
                let Some(c) = bots.bots[i].as_ref() else { continue };
                let b = c.body;
                let me = self.movers[self.bot_fighter[b]].location;
                let dir = c.move_request.map(|m| {
                    let t = self.bot_steer.get(b).copied().flatten().unwrap_or(m.dest);
                    let d = t - me;
                    let l = (d.x * d.x + d.y * d.y).sqrt();
                    if l > 1e-3 { FVector::new(d.x / l, d.y / l, 0.0) } else { FVector::ZERO }
                });
                self.bot_turn[b] = bots.lod_turn(i, dt as f32, dir, now, &mut host);
            }
        }
        // path following (rust-mode-ai r4): each bot's move request steered along the navmesh
        let n = self.bot_fighter.len();
        self.bot_follow.resize_with(n, Default::default);
        self.bot_steer.resize(n, None);
        if let Some(nav) = self.nav.as_mut() {
            for c in bots.bots.iter().flatten() {
                let b = c.body;
                let me = self.movers[self.bot_fighter[b]].location;
                self.bot_steer[b] = match c.move_request {
                    Some(m) => self.bot_follow[b].steer(&mut **nav, me, m.dest),
                    None => {
                        self.bot_follow[b].clear();
                        None
                    }
                };
            }
        }
        self.bots = Some(bots);
    }

    /// Events and log lines emitted since the last call (combat events: hit / parry / chamber / motion / died / ...)
    pub fn drain(&mut self) -> (Vec<Map<String, Value>>, Vec<String>) {
        let e = self.combat.hits[self.hits0..].to_vec();
        let l = self.combat.events[self.log0..].to_vec();
        self.hits0 = self.combat.hits.len();
        self.log0 = self.combat.events.len();
        (e, l)
    }

    /// Snapshot of every fighter: the combat state (mordhau_core::snapshot) + movement (UE cm)
    pub fn snapshot(&self) -> Value {
        let mut out = Map::new();
        for fi in 0..self.combat.fighters.len() {
            let mut s = mordhau_core::snapshot::fighter(&self.combat, fi);
            let m = &self.movers[fi];
            if let Value::Object(o) = &mut s {
                o.insert("location".into(), json!([m.location.x, m.location.y, m.location.z]));
                o.insert("velocity".into(), json!([m.velocity.x, m.velocity.y, m.velocity.z]));
                o.insert("yaw".into(), json!(self.yaw[fi]));
                o.insert("falling".into(), json!(m.mode == Mode::Falling));
                // the final local pose: per bone [qx, qy, qz, qw, x, y, z] (UE, parent-relative), bone order = "bones"
                let pose: Vec<Value> = self.local_pose(fi).iter().map(|x| json!([x.rot.x, x.rot.y, x.rot.z, x.rot.w, x.loc.x, x.loc.y, x.loc.z])).collect();
                o.insert("pose".into(), Value::Array(pose));
            }
            out.insert(self.combat.fighters[fi].name.clone(), s);
        }
        json!({"n": self.combat.tick_n, "t": self.combat.now, "bones": self.geo.skeleton.names, "f": out, "ragdolls": self.ragdolls.as_ref().map(|r|r.debug())})
    }
}

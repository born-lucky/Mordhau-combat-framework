//! MotionSystem: one fighter = UMotionSystemComponent (current motion, NetMotion, motion switching) plus the parts
//! of AMordhauCharacter, UStaminaStatComponent and UHealthStatComponent the combat motions read and write
//! (godot/game/combat/motion_system.gd; sources extract/native/decomp/UMotionSystemComponent.cpp,
//! AMordhauCharacter.cpp, UStaminaStatComponent.cpp, UStatComponent.cpp). One machine, authority role
//! (ROLE_Authority = 3): no prediction, no ping, ExpectedDelay 0. The net hooks (role, remote_controlled) are kept
//! for mh-net.

use super::enums::{self, at, bt, mv, net, ps};
use super::equipment::{EquipmentId, EquipmentPlacement};
use super::motion::{Motion, MotionId, MotionKind};
use super::tracer::Tracer;
use super::world::World;
use crate::data::{self, Character, EquipmentDef, MotionDef, Stat, WeaponData};
use crate::ue::{clampf, f32r, maxf, trunc_i, Xform};
use serde_json::json;
use std::collections::HashMap;
use std::rc::Rc;

/// FNetMotion (types/FNetMotion.h). One per fighter (UMotionSystemComponent +0xc4 NetMotion).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct NetMotion {
    pub id: i64,          // Id (+0x0): AssignNetMotion rva=0x14b34c0 sets NetMotion.Id + 1
    pub motion_type: i64, // enums::net
    pub param0: i64,
    pub param1: i64,
    pub param2: i64,
    pub dynamic_param: i64,
}

impl NetMotion {
    pub fn new(t: i64, p0: i64, p1: i64, p2: i64, dyn_: i64) -> NetMotion {
        NetMotion { id: 0, motion_type: t, param0: p0 & 0xff, param1: p1 & 0xff, param2: p2 & 0xff, dynamic_param: dyn_ & 0xff }
    }
}

pub struct Fighter {
    pub name: String,
    pub id: u32, // this pawn's identity for cross-fighter lists (hit lists, ignore caches)
    pub right_actor: Option<EquipmentId>,
    pub left_actor: Option<EquipmentId>,
    pub inventory_actors: Vec<EquipmentId>,
    pub fists_actor: Option<EquipmentId>,
    pub kick_actor: Option<EquipmentId>,
    pub weapon_path: String,
    pub weapon: Option<Rc<WeaponData>>,         // RightHandEquipment's combat numbers
    pub weapon_equip: Option<Rc<EquipmentDef>>, // RightHandEquipment's equipment fields
    pub motion_bps: HashMap<i64, String>,       // EAttackMove -> motion Blueprint path
    pub parry_bp: String,
    pub left_hand_path: String, // AMordhauCharacter +0x1200 LeftHandEquipment
    pub left_hand: Option<Rc<EquipmentDef>>,
    pub left_weapon: Option<Rc<WeaponData>>,
    pub character: Rc<Character>,
    pub kick_weapon: Option<Rc<WeaponData>>, // AMordhauCharacter +0x1210 KickWeapon
    pub has_last_chance: bool,               // AAdvancedCharacter +0x9f0 bHasLastChance
    pub stamina_stat: Stat,
    pub health_stat: Stat,
    pub tracer: Tracer,

    pub motions: Vec<Option<Motion>>, // motion slab (MotionId = index)
    pub free_slots: Vec<u32>,
    pub motion: Option<MotionId>,              // +0xe8 Motion
    pub last_attack_motion: Option<MotionId>,  // +0x108
    pub last_feinted_motion: Option<MotionId>, // +0x100
    pub last_parry_motion: Option<MotionId>,   // +0xf0
    pub net: NetMotion,                        // +0xc4 NetMotion
    pub next_kick_time: f64,                   // +0xb0
    pub next_attack_time: f64,                 // +0xb4
    pub next_available_stun_time: f64,         // +0xb8
    pub stamina: i64,                          // stamina UStatComponent +0xb0 StatValue
    pub next_stamina_regen: f64,               // stamina UStatComponent +0xb4 NextRegenerationTick
    pub stamina_regenerable: bool,             // bIsRegenerable (cleared on death)
    pub health: i64,                           // health UStatComponent +0xb0 StatValue
    pub easy_parry_until_time: f64,            // AMordhauCharacter +0xe94
    pub armor_tier_override: i64,              // AAdvancedCharacter +0x7f4 DamageArmorTierOverride
    pub wearable_coverage: Vec<(String, i64)>, // AMordhauCharacter +0xb18 WearableProtectionCoverageMap: bone -> ArmorClass
    pub airborne: bool,                        // AAdvancedCharacter +0x839 ReplicatedCharacterFlags bit 0
    pub airborne_time: f64,                    // AAdvancedCharacter +0x86c AirborneTime
    pub holding_block: bool,                   // AMordhauCharacter +0xb99 bIsHoldingBlock
    pub wants_block: bool,                     // AMordhauCharacter +0x1029 bWantsBlock
    pub look_up_value: f64,                    // AAdvancedCharacter +0x520 LookUpValue, degrees
    pub turn_caps: super::turncap::TurnCaps,   // AAdvancedCharacter TurnRateCap / TurnCapRemaining / ... (turncap.rs)
    pub wants_fire: bool,                      // AMordhauCharacter bWantsFire (rangedmotion.rs)
    /// the pawn's velocity (UE cm/s; host-fed: ProcessHitForDamage's mounted speed factor)
    pub velocity: crate::ue::FVector,
    /// riding a horse (CurrentVehicle is an AHorse; host-fed, horse.rs Mount)
    pub mount: Option<super::horse::Mount>,
    /// the ranged equipment in hand (rangedmotion.rs); None = no ranged weapon (set by the host)
    pub ranged: Option<super::rangedmotion::RangedEquip>,
    /// AMordhauCharacter LastRequestedFireOrigin / LastRequestedFireRotation (ServerAssignFireAim)
    pub last_fire_aim: (crate::ue::FVector, (f32, f32, f32)),
    pub b_is_left_arm_disabled: bool,          // +0xd40
    pub b_is_right_arm_disabled: bool,         // +0xd41
    pub alternate_mode: bool,
    pub preset_flip: bool, // AMordhauCharacter +0xd78 bWantsFlipAttackSide
    pub bone_xf: HashMap<String, Xform>, // bone -> world transform (Godot convention), fed by the host
    pub actor_xf: Option<Xform>,         // actor (capsule) world transform; None = unknown (headless)
    pub dead: bool,
    pub role: i64, // ENetRole: 1 simulated, 2 autonomous, 3 authority
    /// AAdvancedCharacter +0x4e8, IsViewTarget (RVA8e1fb0), independent of perspective/animation.
    pub is_view_target: bool,
    pub remote_controlled: bool,
    pub team: i64,          // AMordhauPlayerState Team, 255 = none
    pub creation_time: f64, // AActor +0x9c CreationTime
    /// UE-space geometry from the host (None: no geometry rules, the reference's contact model)
    pub geom: Option<super::world::FighterGeom>,
    /// AMordhauCharacter bIsBlockColliderEnabled (EnableBlockCollider rva=0x153a550 / DisableBlockCollider rva=0x1538cc0,
    /// byte-matched src/Mordhau/Private/MordhauCharacter.cpp); set by attack / parry OnBegin, cleared on leave (exe mode)
    pub block_collider_enabled: bool,
    /// the held weapon's ClashCollider collides (AMordhauWeapon::OnAttackStarted_Implementation rva=0x162eb40 enables
    /// it QueryOnly; exe mode: while an attack is current)
    pub clash_collider_enabled: bool,
    /// net data combat reads while creating a motion (net.rs); default = offline
    pub net_slot: super::net::NetSlot,
    /// mh-net's per-fighter replication state (NetMotionRep), None offline
    pub net_state: Option<Box<dyn std::any::Any>>,
}

pub const DAMAGE_MELEE: i64 = 1; // EMordhauDamageType (UDamageableComponent::OnPostTakeDamage disasm 0x141490c26)
pub const DAMAGE_RANGED: i64 = 2;
pub const DAMAGE_FALL: i64 = 3; // types/EMordhauDamageType.h

fn file_of(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

impl World {
    pub fn f(&self, fi: usize) -> &Fighter {
        &self.fighters[fi]
    }
    pub fn fm(&mut self, fi: usize) -> &mut Fighter {
        &mut self.fighters[fi]
    }
    pub fn cur(&self, fi: usize) -> Option<MotionId> {
        self.fighters[fi].motion
    }
    pub fn cur_m(&self, fi: usize) -> Option<&Motion> {
        self.fighters[fi].motion.map(|id| self.m(fi, id))
    }

    /// MotionSystem._init: weapon setup, left hand, character + stat defaults, KickWeapon, then ChangeMotion(Idle)
    pub(crate) fn new_fighter(&mut self, name: &str, weapon_path: &str, left_path: &str) -> usize {
        let spec = self.spec.clone();
        let s = spec.weapon_setup(weapon_path);
        let character = spec.character.clone();
        let stamina_stat = spec.stat("UStaminaStatComponent").clone();
        let health_stat = spec.stat("UHealthStatComponent").clone();
        self.next_fighter_id += 1;
        let (left_hand, left_weapon) = if !left_path.is_empty() {
            let lh = spec.equip(left_path);
            let lw = if lh.as_ref().map(|e| e.is_weapon).unwrap_or(false) { spec.weapon(left_path) } else { None };
            (lh, lw)
        } else {
            (None, None)
        };
        let kp = &spec.kick_weapon_path;
        let pawn = self.next_fighter_id;
        let right_actor = self.create_pawn_equipment(pawn, weapon_path).expect("spawn right equipment");
        let left_actor = if left_hand.is_some() {
            Some(self.create_pawn_equipment(pawn, left_path).expect("spawn left equipment"))
        } else { None };
        let kick_actor = if kp.is_empty() { None } else {
            Some(self.create_pawn_equipment(pawn, kp).expect("spawn kick equipment"))
        };
        self.equipment.resolve_mut(right_actor).unwrap().placement = EquipmentPlacement::RightHand;
        if let Some(id) = left_actor { self.equipment.resolve_mut(id).unwrap().placement = EquipmentPlacement::LeftHand; }
        if let Some(id) = kick_actor { self.equipment.resolve_mut(id).unwrap().placement = EquipmentPlacement::VirtualWeapon; }
        let fists_actor = if self.equipment_is_class(right_actor, "AFistsWeapon") { Some(right_actor) } else { None };
        let f = Fighter {
            name: name.to_string(),
            id: self.next_fighter_id,
            right_actor: Some(right_actor), left_actor, kick_actor, fists_actor,
            inventory_actors: [Some(right_actor), left_actor].into_iter().flatten().collect(),
            weapon_path: weapon_path.to_string(),
            weapon: Some(s.weapon.clone()),
            weapon_equip: s.equip.clone(),
            motion_bps: s.motions.clone(),
            parry_bp: String::new(),
            left_hand_path: left_path.to_string(),
            left_hand,
            left_weapon,
            kick_weapon: if kp.is_empty() { None } else { spec.weapon(kp) },
            has_last_chance: character.b_has_last_chance,
            armor_tier_override: character.damage_armor_tier_override,
            // UStaminaStatComponent::InitializeComponent rva=0x14fdd40 / UStatComponent::InitializeComponent
            // rva=0x14fde40: StatValue = InitialStatValue
            stamina: stamina_stat.initial_value,
            health: health_stat.initial_value,
            stamina_stat,
            health_stat,
            character,
            tracer: Tracer::new(&s.tracer, &spec.constants),
            motions: Vec::new(),
            free_slots: Vec::new(),
            motion: None,
            last_attack_motion: None,
            last_feinted_motion: None,
            last_parry_motion: None,
            net: NetMotion::default(),
            next_kick_time: 0.0,
            next_attack_time: 0.0,
            next_available_stun_time: 0.0,
            next_stamina_regen: 0.0,
            stamina_regenerable: true,
            easy_parry_until_time: 0.0,
            wearable_coverage: Vec::new(),
            airborne: false,
            airborne_time: 0.0,
            holding_block: false,
            wants_block: false,
            look_up_value: 0.0,
            turn_caps: Default::default(),
            wants_fire: false,
            velocity: crate::ue::FVector::ZERO,
            mount: None,
            ranged: None,
            last_fire_aim: (crate::ue::FVector::ZERO, (0.0, 0.0, 0.0)),
            b_is_left_arm_disabled: false,
            b_is_right_arm_disabled: false,
            alternate_mode: false,
            preset_flip: false,
            bone_xf: HashMap::new(),
            actor_xf: None,
            dead: false,
            role: 3,
            is_view_target: false,
            remote_controlled: false,
            team: 255,
            creation_time: 0.0,
            geom: None,
            block_collider_enabled: false,
            clash_collider_enabled: false,
            net_slot: Default::default(),
            net_state: None,
        };
        self.fighters.push(f);
        let fi = self.fighters.len() - 1;
        self.project_equipment_hands(fi, false);
        self.change_to_idle(fi);
        fi
    }

    // ---- motion slab ----
    pub(crate) fn alloc_motion(&mut self, fi: usize, m: Motion) -> MotionId {
        let f = &mut self.fighters[fi];
        if let Some(i) = f.free_slots.pop() {
            f.motions[i as usize] = Some(m);
            MotionId(i)
        } else {
            f.motions.push(Some(m));
            MotionId(f.motions.len() as u32 - 1)
        }
    }

    pub(crate) fn new_motion(&mut self, fi: usize, def: Rc<MotionDef>, kind: &str) -> MotionId {
        use super::attack::AttackMotion;
        use super::blocked::BlockedMotion;
        use super::feinted::FeintedMotion;
        use super::parry::ParryMotion;
        use super::react::{DisarmedMotion, FlinchMotion, StunMotion};
        let k = match kind {
            "Idle" => MotionKind::Idle,
            "Attack" => MotionKind::Attack(Box::new(AttackMotion::default())),
            "Parry" => MotionKind::Parry(Box::new(ParryMotion::default())),
            "Feinted" => MotionKind::Feinted(Box::new(FeintedMotion::default())),
            "Blocked" => MotionKind::Blocked(Box::new(BlockedMotion::default())),
            "Flinch" => MotionKind::Flinch(Box::new(FlinchMotion::new())),
            "Stun" => MotionKind::Stun(Box::new(StunMotion::default())),
            "Disarmed" => MotionKind::Disarmed(Box::new(DisarmedMotion::default())),
            // rust-net r5: a networked climb is created by HandleNetMotionUpdate (begin_net_motion, net::CLIMBING)
            "Climbing" => MotionKind::Climbing(Box::new(super::climb::ClimbingMotion::ctor())),
            "ModeSwitch" => MotionKind::ModeSwitch(Box::new(super::modeswitch::ModeSwitchMotion { b_is_switching_to_alt: self.fighters[fi].net.param0 != 0, ..Default::default() })),
            _ => panic!("motion kind {kind}"),
        };
        self.alloc_motion(fi, Motion::new(def, k))
    }

    /// UMotionSystemComponent::ChangeMotion / ChangeMotion_Internal rva=0x14b4f20 (decomp 1719-1745): Motion (+0xe8)
    /// = new first, then new ComingFromMotion = old, then old UMordhauMotion::Interrupt rva=0x165d5e0 (LeaveTime = now,
    /// OnLeave(true)) - so the old motion's OnLeave already sees the new one as current -, then
    /// UMordhauMotion::Initialize rva=0x165d4c0: StartTime = now, EndTime = StartTime, OnBegin, and OnTick with
    /// DeltaTime 0 (disasm 0x14165d5c5 call OnBegin, 0x14165d5ca `xorps xmm1, xmm1`, tail jump to OnTick rva=0x16d6990).
    pub(crate) fn change(&mut self, fi: usize, m: MotionId) {
        let now = self.now;
        let old = self.fighters[fi].motion;
        self.fighters[fi].motion = Some(m);
        self.mm(fi, m).coming_from = old;
        if let Some(o) = old {
            self.mm(fi, o).leave_time = now;
            self.motion_on_leave(fi, o, true);
        }
        let nm = self.mm(fi, m);
        nm.start_time = now;
        nm.end_time = now;
        // game/net: bInitiatedLocally / bWasConfirmedByAuthority / ExpectedDelay, set before Initialize
        // (NetMotionRep.init_motion in MotionSystem._change); offline: no pending values -> 0
        let d = self.fighters[fi].net_slot.init_motion(m);
        self.mm(fi, m).expected_delay = d;
        self.motion_on_begin(fi, m);
        if self.m(fi, m).is_attack() {
            // UAttackMotion::OnBegin_Implementation stores it near its end, before OnTick
            self.fighters[fi].last_attack_motion = Some(m);
        }
        self.motion_on_tick(fi, m, 0.0); // on `m` even when its OnBegin already changed the motion
        self.bound_history(fi);
        self.trace_motion(fi);
    }

    fn links(&self, fi: usize, id: MotionId) -> Vec<MotionId> {
        let m = self.m(fi, id);
        let mut out = Vec::new();
        if let Some(c) = m.coming_from {
            out.push(c);
        }
        match &m.k {
            MotionKind::Attack(a) => out.extend(a.previous_last_attack),
            MotionKind::Blocked(b) => out.extend(b.from_attack),
            _ => {}
        }
        out
    }

    fn roots(&self, fi: usize) -> Vec<MotionId> {
        let f = &self.fighters[fi];
        [f.motion, f.last_attack_motion, f.last_parry_motion, f.last_feinted_motion].into_iter().flatten().collect()
    }

    /// Memory bound of the motion history (MotionSystem._bound_history, a port-design rule with no exe counterpart:
    /// UE's GC frees unreachable motions): the links of every motion two hops from a root are cut. Roots = Motion
    /// +0xe8, LastParryMotion +0xf0, LastFeintedMotion +0x100, LastAttackMotion +0x108.
    fn bound_history(&mut self, fi: usize) {
        let roots = self.roots(fi);
        for r in &roots {
            for h1 in self.links(fi, *r) {
                for h2 in self.links(fi, h1) {
                    if !roots.contains(&h2) {
                        let m = self.mm(fi, h2);
                        m.coming_from = None;
                        match &mut m.k {
                            MotionKind::Attack(a) => a.previous_last_attack = None,
                            MotionKind::Blocked(b) => b.from_attack = None,
                            _ => {}
                        }
                    }
                }
            }
        }
    }

    /// Frees the slab slots no root reaches any more (the refcount release of the reference). Run between steps only,
    /// never inside a rule, so a motion id held by a running rule stays valid like a GDScript local reference.
    pub(crate) fn collect_motions(&mut self, fi: usize) {
        let mut keep = vec![false; self.fighters[fi].motions.len()];
        let mut stack = self.roots(fi);
        while let Some(id) = stack.pop() {
            if keep[id.0 as usize] {
                continue;
            }
            keep[id.0 as usize] = true;
            stack.extend(self.links(fi, id));
        }
        let f = &mut self.fighters[fi];
        for (i, k) in keep.iter().enumerate() {
            if !k && f.motions[i].is_some() {
                f.motions[i] = None;
                f.free_slots.push(i as u32);
            }
        }
    }

    pub fn change_to_idle(&mut self, fi: usize) {
        let d = self.spec.native_motion_def("UIdleMotion");
        let id = self.new_motion(fi, d, "Idle");
        self.change(fi, id);
    }

    /// UMotionSystemComponent::GetAttackMotionClass rva=0x14bc010: profile Attacks[move], else UAttackMotion
    pub fn attack_motion_defaults(&self, fi: usize, m: i64) -> Rc<MotionDef> {
        match self.attack_motion_bp(fi, m) {
            Some(p) => self.spec.motion_def(&p),
            None => self.spec.native_motion_def("UAttackMotion"),
        }
    }

    /// The attack motion Blueprint GetAttackMotionClass rva=0x14bc010 picks (state/proofs/shield_motions.md): the
    /// right-hand equipment's profile Attacks[move] (decomp UMotionSystemComponent.cpp 459-518; `motion_bps`, which
    /// also holds the component map's Kick), then the LEFT-hand equipment's profile when it is a weapon (a shield:
    /// BP_ShieldAnimationProfile), whose entry replaces it (LAB_1414bc1f5, decomp 520-575)
    pub fn attack_motion_bp(&self, fi: usize, m: i64) -> Option<String> {
        let f = &self.fighters[fi];
        if self.left_is_weapon(fi) {
            if let Some(prof) = f.left_hand.as_ref().map(|e| e.weapon_animation_profile.clone()).filter(|p| !p.is_empty()) {
                if let Some(p) = self.spec.profile_motions(&prof).motions.get(&m).filter(|p| !p.is_empty()) {
                    return Some(p.clone());
                }
            }
        }
        f.motion_bps.get(&m).filter(|p| !p.is_empty()).cloned()
    }

    /// UAttackMotion::FindWeapon_Implementation rva=0x161d9f0: the RightHandEquipment; UKickMotion::FindWeapon_
    /// Implementation rva=0x165b4b0: the character's KickWeapon (+0x1210)
    pub fn find_weapon(&self, fi: usize, native: &str) -> Option<Rc<WeaponData>> {
        if self.spec.is_class_of(native, "UKickMotion") {
            return self.fighters[fi].kick_weapon.clone();
        }
        self.fighters[fi].weapon.clone()
    }

    /// Actor counterpart of FindWeapon: a capture cannot be reconstructed later from shared class-data Rcs.
    pub fn find_weapon_actor(&self, fi: usize, native: &str) -> Option<EquipmentId> {
        let f = &self.fighters[fi];
        let id = if self.spec.is_class_of(native, "UKickMotion") { f.kick_actor } else { f.right_actor };
        id.filter(|id| self.equipment_actor(*id).is_some())
    }

    /// UMotionSystemComponent::CanPerformAttack rva=0x14b4df0 -> AMordhauEquipment::CanPerformAttack_Implementation
    /// rva=0x15328a0: !bCanAttack (+0xcb9) -> false; Couch with Stamina byte (+0xe67) == 0 -> false; on foot:
    /// !bCanAttackOnFoot -> false; Kick while airborne without bCanJumpKick -> false. (UNCONFIRMED naming:
    /// MovementMode 3 = airborne.)
    pub fn can_perform_attack(&self, fi: usize, m: i64) -> bool {
        let f = &self.fighters[fi];
        let Some(w) = &f.weapon else { return false };
        if !w.b_can_attack {
            return false;
        }
        if m == mv::COUCH && self.stamina_byte(fi) == 0 {
            return false;
        }
        // AMordhauEquipment::CanPerformAttack_Implementation rva=0x15328a0: on foot bCanAttackOnFoot, on a horse
        // bCanAttackOnHorseback
        if if f.mount.is_some() { !w.b_can_attack_on_horseback } else { !w.b_can_attack_on_foot } {
            return false;
        }
        if m == mv::KICK && f.airborne && !f.character.b_can_jump_kick {
            return false;
        }
        true
    }

    /// The drop of UDisarmedMotion::OnBegin_Implementation rva=0x165ee70 / UStunMotion::OnBegin_Implementation
    /// rva=0x16628e0 (bWillDisarm), authority only: ToDrop = the ComingFromMotion's Weapon (+0x10c8) when it IsA
    /// UAttackMotion, else LeftHandEquipment when it IsA AMordhauWeapon, else RightHandEquipment;
    /// AMordhauCharacter::DropEquipment; then with no RightHandEquipment, SwitchToFists rva=0x156e920 (the
    /// `switch_to_fists` event asks the host, which owns the loadout, to transfer its designated fists). UNCONFIRMED: the
    /// SwitchEquipment motion's timing (not ported).
    pub(crate) fn drop_for_disarm(&mut self, fi: usize, from: Option<MotionId>) {
        if !self.authority {
            return;
        }
        // Native first chooses left weapon/right weapon; a non-null immediate attack Weapon overrides it
        // (UDisarmedMotion.cpp:140-185, UStunMotion.cpp:156-201). A stale capture must not select another actor.
        let from_actor = from.and_then(|id| self.m(fi, id).attack().and_then(|a| a.weapon_actor));
        let f = &self.fighters[fi];
        let actor = from_actor.or_else(|| if self.left_is_weapon(fi) { f.left_actor } else { f.right_actor });
        let Some(actor) = actor else { return };
        let Some(a) = self.equipment_actor(actor) else { return };
        // VirtualWeapon ctor (rva=0x166fa90, AVirtualWeapon.cpp:26) forbids dropping, including KickWeapon.
        // Do not replace a non-droppable attack origin with a different held actor.
        if a.weapon.as_ref().is_none_or(|w| !w.b_allow_drop) { return; }
        let path = a.path.clone();
        let hand = if f.right_actor == Some(actor) { "right" } else if f.left_actor == Some(actor) { "left" } else { "holstered" };
        if self.drop_actor(actor).is_err() { return; }
        let name = self.fighters[fi].name.clone();
        self.trace_event(&format!("{name} dropped {hand} {}", file_of(&path)));
        self.emit_event(json!({"kind": "drop_equipment", "who": name, "hand": hand, "path": path}));
        if self.fighters[fi].weapon.is_none() {
            self.emit_event(json!({"kind": "switch_to_fists", "who": name}));
        }
    }

    /// from UMotionSystemComponent::AssignNetAttackMotion rva=0x14b3210
    pub fn assign_net_attack_motion(&mut self, fi: usize, ty: i64, m: i64, angle: f64) {
        if !self.can_perform_attack(fi, m) {
            return;
        }
        let t = self.now;
        // ping compensation (AutonomousProxy only; 0 on the authority): GetPing x TimeDilation against the motion's
        // GetAttackCompensationStartTime (net.rs NetHooks::attack_ping_compensation)
        let comp = match self.net.clone() {
            Some(h) => h.attack_ping_compensation(self, fi, m),
            None => 0.0,
        };
        let c = &self.spec.constants;
        let q = self.qf();
        let mut kick = 0.0;
        if m == mv::KICK {
            let k3 = c.kick_lag_window;
            kick = q(k3 - clampf(q(self.fighters[fi].next_kick_time - t), 0.0, k3));
        }
        // single precision as in AssignNetAttackMotion at 0x1414b33e8 (addss), 0x1414b341c (mulss), 0x1414b3460
        // (cvttss2si): 0.08 x 500 = 40, not 39
        let mut lag_f = f32r(clampf(comp, 0.0, c.max_ping_compensation) + kick);
        lag_f = f32r(lag_f * c.net_lag_units_per_s);
        let lag = trunc_i(lag_f) & 0xff;
        let a = q(clampf(q(angle), c.attack_angle_min, c.attack_angle_max) * c.attack_angle_to_unit);
        let p1 = self.pack_float(a, -1.0, 1.0);
        self.assign(fi, NetMotion::new(net::ATTACK, enums::pack(ty, m), p1, lag, 0));
    }

    /// UMordhauUtilityLibrary::PackFloat rva=0x14cfde0: int(clamp01((v - lo) / (hi - lo)) * 255), degenerate range ->
    /// 255 if v >= hi else 0
    pub fn pack_float(&self, v: f64, lo: f64, hi: f64) -> i64 {
        let c = &self.spec.constants;
        let q = self.qf();
        let span = q(hi - lo);
        let mut f = 0.0;
        if span.abs() > c.small_number {
            f = q(q(v - lo) / span);
        } else if hi <= v {
            return trunc_i(c.byte_max);
        }
        trunc_i(q(clampf(f, 0.0, 1.0) * c.byte_max))
    }

    /// FNetMotion::Parry(EBlockType) as built by UMordhauMotion::ProcessBlock_Implementation: MotionType 2
    pub fn assign_net_parry(&mut self, fi: usize, b: i64) {
        self.assign(fi, NetMotion::new(net::PARRY, b, 0, 0, 0));
    }
    /// FNetMotion::Feinted(EFeintType, EAttackMove) as built by UAttackMotion::ProcessFeint_Implementation
    pub fn assign_net_feint(&mut self, fi: usize, ft: i64, m: i64) {
        self.assign(fi, NetMotion::new(net::FEINTED, ft, m, 0, 0));
    }
    /// FNetMotion::Blocked rva=0x1615f50: Param0 reason, Param1 FBlockResult bools, Param2 = time x 40 clamped to
    /// a byte (UAttackMotion::PutUsInBlockedMotionFromParry rva=0x163a7d0)
    pub fn assign_net_blocked(&mut self, fi: usize, reason: i64, flags: i64, time_s: f64) {
        let c = &self.spec.constants;
        let b = clampf(self.qf()(time_s * c.blocked_time_to_byte), 0.0, c.byte_max);
        self.assign(fi, NetMotion::new(net::BLOCKED, reason, flags, trunc_i(b), 0));
    }
    /// FNetMotion::Stunned rva=0x14d86a0: Param0 = PackFloat(direction x 1/180, [-1, 1]), Param1 bone, Param2 disarm
    pub fn assign_net_stunned(&mut self, fi: usize, direction: f64, bone: i64, should_disarm: bool) {
        let p0 = self.pack_float(self.qf()(direction * self.spec.constants.flinch_angle_to_unit), -1.0, 1.0);
        self.assign(fi, NetMotion::new(net::STUNNED, p0, bone, should_disarm as i64, 0));
    }
    /// FNetMotion::Disarmed rva=0x14ba4a0: Param0 = PackFloat(direction x 1/180, [-1, 1])
    pub fn assign_net_disarmed(&mut self, fi: usize, direction: f64) {
        let p0 = self.pack_float(self.qf()(direction * self.spec.constants.flinch_angle_to_unit), -1.0, 1.0);
        self.assign(fi, NetMotion::new(net::DISARMED, p0, 0, 0, 0));
    }
    /// FNetMotion::Flinched rva=0x14bb820: Param0 = PackFloat(angle / 180), Param1 flag, Param2 =
    /// PackFloat(FlinchDurationModifier, [0.5, 2]), DynamicParam = PackFloat(FlinchSpeedModifier, [0, 1])
    pub fn assign_net_flinched(&mut self, fi: usize, angle: f64, flag: bool, duration_mod: f64, speed_mod: f64) {
        let p0 = self.pack_float(self.qf()(angle * self.spec.constants.flinch_angle_to_unit), -1.0, 1.0);
        let p2 = self.pack_float(duration_mod, 0.5, 2.0);
        let d = self.pack_float(speed_mod, 0.0, 1.0);
        self.assign(fi, NetMotion::new(net::FLINCHED, p0, flag as i64, p2, d));
    }

    /// UMotionSystemComponent::AssignNetMotion rva=0x14b34c0: NewNetMotion.Id = NetMotion.Id + 1; alone (no mh-net),
    /// the authority's HandleNetMotionUpdate reduces to "NetMotion = new, create its motion".
    pub(crate) fn assign(&mut self, fi: usize, mut nm: NetMotion) {
        nm.id = (self.fighters[fi].net.id + 1) & 0xff;
        if let Some(h) = self.net.clone() {
            // the role-dependent rest of AssignNetMotion (ServerAssignNetMotion / ClientSetNetMotion /
            // HandleNetMotionUpdate) runs in mh-net
            h.assign(self, fi, nm);
            return;
        }
        self.fighters[fi].net = nm;
        self.begin_net_motion(fi);
    }

    /// The motion-creating tail of UMotionSystemComponent::HandleNetMotionUpdate rva=0x14c01b0: NewObject of the
    /// class for MotionType (AMordhauCharacter::Motions +0x10d0 indexed by MotionType, disasm 0x1414c0459..
    /// 0x1414c04a9; BP_MordhauCharacter CDO entries), ChangeMotion_Internal, then DynamicParamChanged(0, dyn) when != 0.
    pub fn begin_net_motion(&mut self, fi: usize) {
        let nm = self.fighters[fi].net;
        let spec = self.spec.clone();
        let (def, kind) = match nm.motion_type {
            net::ATTACK => (self.attack_motion_defaults(fi, enums::lo(nm.param0)), "Attack"),
            net::PARRY => {
                let pb = &self.fighters[fi].parry_bp;
                (if pb.is_empty() { spec.native_motion_def("UParryMotion") } else { spec.motion_def(pb) }, "Parry")
            }
            net::FEINTED => (spec.motion_def(&data::feinted_motion()), "Feinted"),
            net::BLOCKED => (spec.motion_def(&data::blocked_motion()), "Blocked"),
            net::FLINCHED => (spec.motion_def(&data::flinch_motion()), "Flinch"),
            net::STUNNED => (spec.motion_def(&data::stun_motion()), "Stun"),
            net::DISARMED => (spec.motion_def(&data::disarmed_motion()), "Disarmed"),
            // rust-net r5: FNetMotion::Climbing (type 0x1b) -> UClimbingMotion (climb.rs climbing_def)
            net::CLIMBING => (super::climb::climbing_def(), "Climbing"),
            // fp-anim r3: UEquipmentModeSwitchMotion (modeswitch.rs)
            net::EQUIPMENT_MODE_SWITCH => (super::modeswitch::mode_switch_def(), "ModeSwitch"),
            _ => return,
        };
        let id = self.new_motion(fi, def, kind);
        if let MotionKind::Climbing(c) = &mut self.mm(fi, id).k {
            // OnBegin reads the climb offset / bIsSlowClimb from the motion system's NetMotion params (+0xc6..+0xc9)
            c.params = [nm.param0 as u8, nm.param1 as u8, nm.param2 as u8, nm.dynamic_param as u8];
        }
        self.change(fi, id);
        let d = self.fighters[fi].net.dynamic_param;
        if d != 0 {
            if let Some(c) = self.fighters[fi].motion {
                self.motion_on_dynamic_param_changed(fi, c, 0, d);
            }
        }
    }

    /// from UMotionSystemComponent::AssignNetMotionDynamicParam rva=0x14b35f0 (authority: set, notify the motion)
    pub fn assign_net_motion_dynamic_param(&mut self, fi: usize, v: i64) {
        if let Some(h) = self.net.clone() {
            h.assign_dynamic_param(self, fi, v); // Role 3 only, then ClientSetNetMotion + OnRep (mh-net)
            return;
        }
        let old = self.fighters[fi].net.dynamic_param;
        self.fighters[fi].net.dynamic_param = v & 0xff;
        let nv = self.fighters[fi].net.dynamic_param;
        if old != nv {
            if let Some(c) = self.fighters[fi].motion {
                self.motion_on_dynamic_param_changed(fi, c, old, nv);
            }
        }
        let s = format!("{} dyn {}", self.fighters[fi].name, self.fighters[fi].net.dynamic_param);
        self.trace_event(&s);
    }

    /// AMordhauCharacter::ServerDropParry_Implementation rva=0x1567ad0 (disasm 0x141567ad0..0x141567b3a): return if
    /// bIsDead, if NetMotion.Id != MotionID, or if the current Motion is not a UParryMotion; else tail-jump to
    /// UParryMotion::StopHolding. On the authority it runs here directly.
    pub fn server_drop_parry(&mut self, fi: usize) {
        let id = self.fighters[fi].net.id;
        if self.fighters[fi].role != 3 {
            if let Some(h) = self.net.clone() {
                h.send_server_drop_parry(self, fi, id); // the reliable server RPC (mh-net)
                return;
            }
        }
        self.server_drop_parry_implementation(fi, id);
    }

    pub fn server_drop_parry_implementation(&mut self, fi: usize, motion_id: i64) {
        let f = &self.fighters[fi];
        if f.dead || f.net.id != motion_id {
            return;
        }
        let Some(c) = f.motion else { return };
        if !self.m(fi, c).is_parry() {
            return;
        }
        self.parry_stop_holding(fi, c);
    }

    /// The gate in front of UParryMotion::OnTick_Implementation's ServerDropParry call (rva=0x1668980, decomp
    /// UParryMotion.cpp 375-390): IsLocallyControlled (vcall +0x9c8), or Role == 3 with no Controller (+0x9c0)
    pub fn drops_parry_locally(&self, fi: usize) -> bool {
        let f = &self.fighters[fi];
        f.role == 2 || (f.role == 3 && !f.remote_controlled)
    }
    /// APawn::IsLocallyControlled (vcall +0x9c8) in the role model above
    pub fn is_locally_controlled(&self, fi: usize) -> bool {
        self.drops_parry_locally(fi)
    }

    /// from UStaminaStatComponent::OffsetStamina rva=0x1507c20 (costs scaled by StaminaCostModifier +0xe44) and
    /// UStaminaStatComponent::SetStatValue_Internal rva=0x1515c80 (clamp: < Min -> Min, >= Max -> Max); authority only.
    pub fn offset_stamina(&mut self, fi: usize, mut v: i64) {
        let q = self.qf();
        let f = &self.fighters[fi];
        let m = f.character.stamina_cost_modifier;
        if v < 0 && m != 1.0 {
            v = trunc_i(q(v as f64 * m));
        }
        if f.role != 3 {
            return;
        }
        let nv = f.stamina + v;
        let old = f.stamina;
        self.set_stamina(fi, nv);
        // SetStatValue_Internal(v, bReplicate true) writes the ReplicatedStamina byte when the value changed
        if self.fighters[fi].stamina != old {
            if let Some(h) = self.net.clone() {
                h.stat_written(self, fi, false);
            }
        }
        let s = format!("{} stamina {:+} -> {}", self.fighters[fi].name, v, self.fighters[fi].stamina);
        self.trace_event(&s);
    }

    /// SetStatValue_Internal(v, bReplicate false) of the stamina stat (OnRep_ReplicatedStamina's path)
    pub fn set_stamina_value(&mut self, fi: usize, nv: i64) {
        self.set_stamina(fi, nv);
    }

    fn set_stamina(&mut self, fi: usize, nv: i64) {
        let f = &mut self.fighters[fi];
        let (mn, mx) = (f.stamina_stat.min_value, f.stamina_stat.max_value);
        f.stamina = if nv < mn { mn } else if nv < mx { nv } else { mx };
    }

    /// AMordhauCharacter +0xe67 Stamina byte = RoundToInt(100 * (StatValue - Min) / (Max - Min)) (SetStatValue_Internal)
    pub fn stamina_byte(&self, fi: usize) -> i64 {
        let f = &self.fighters[fi];
        let mn = f.stamina_stat.min_value as f64;
        let mx = f.stamina_stat.max_value as f64;
        if self.exe() {
            // UStaminaStatComponent::SetStatValue_Internal rva=0x1515c80 (decomp 204-227): GetMappedRangeValueClamped in
            // float (|Max - Min| > 1e-8, else 0 / 1 by StatValue < Max), then (int)ROUND(x*100 + x*100 + 0.5) >> 1
            // (RoundToInt); the reference uses round() half away from zero on doubles (exe.rs difference 5)
            let q = crate::ue::f32r;
            let span = q(mx - mn);
            let s = f.stamina as f64;
            let fr = if span.abs() > 1e-8 { clampf(q(q(s - mn) / span), 0.0, 1.0) } else if s < mx { 0.0 } else { 1.0 };
            let h = q(fr * 100.0);
            return crate::ue::cvtss2si(q(q(h + h) + 0.5)) >> 1;
        }
        let fr = if mx != mn { clampf((f.stamina as f64 - mn) / (mx - mn), 0.0, 1.0) } else { 1.0 };
        trunc_i((fr * 100.0).round())
    }

    /// AMordhauCharacter health byte (UHealthStatComponent::SetStatValue_Internal rva=0x14d5cf0): RoundToInt(100 x
    /// fraction), 1 when that rounds to 0 while StatValue != 0
    pub fn health_byte(&self, fi: usize) -> i64 {
        let f = &self.fighters[fi];
        let mn = f.health_stat.min_value as f64;
        let mx = f.health_stat.max_value as f64;
        let h = f.health as f64;
        let mut fraction = 0.0;
        if (mx - mn).abs() > self.spec.constants.small_number {
            fraction = clampf((h - mn) / (mx - mn), 0.0, 1.0);
        } else if h >= mx {
            fraction = 1.0;
        }
        let p = trunc_i((fraction * 100.0).round());
        if p == 0 && f.health != 0 { 1 } else { p }
    }

    /// AMordhauCharacter::StopStaminaRegen rva=0x156df00 / UStatComponent::StopRegeneration rva=0x1516820:
    /// NextRegenerationTick = max(now + StaminaRegenDelay + extra, NextRegenerationTick)
    pub fn stop_stamina_regen(&mut self, fi: usize, extra: f64) {
        let now = self.now;
        let q = self.qf();
        let f = &mut self.fighters[fi];
        f.next_stamina_regen = maxf(q(q(now + f.character.stamina_regen_delay) + extra), f.next_stamina_regen);
    }

    /// from UStatComponent::TickStat rva=0x1519ab0 with the stamina parameters UStaminaStatComponent::TickStat
    /// rva=0x1519900 copies in (StaminaRegenPerTick +0xe69, StaminaRegenTickRate +0xe78)
    fn tick_stamina_regen(&mut self, fi: usize) {
        let t = self.now;
        let q = self.qf();
        let f = &self.fighters[fi];
        if !f.stamina_stat.b_is_regenerable || !f.stamina_regenerable {
            return;
        }
        let per = f.character.stamina_regen_per_tick;
        let rate = f.character.stamina_regen_tick_rate;
        let (mx, mn) = (f.stamina_stat.max_value, f.stamina_stat.min_value);
        if !((per >= 1 && f.stamina < mx) || (per < 0 && mn < f.stamina)) {
            return;
        }
        if t <= f.next_stamina_regen {
            return;
        }
        loop {
            let nv = self.fighters[fi].stamina + per;
            let nr = q(self.fighters[fi].next_stamina_regen + rate);
            self.fighters[fi].next_stamina_regen = nr;
            if self.fighters[fi].stamina != nv {
                self.set_stamina(fi, nv);
            }
            let s = self.fighters[fi].stamina;
            if (per >= 1 && mx <= s) || (per < 1 && (per >= 0 || s <= mn)) {
                break;
            }
            if t <= self.fighters[fi].next_stamina_regen {
                return;
            }
        }
        self.fighters[fi].next_stamina_regen = q(t + rate);
    }

    /// Melee damage on this character (ProcessHitForDamage's TakeDamage, vtable +0x590):
    /// AMordhauCharacter::TakeDamage rva=0x156e980 -> AAdvancedCharacter::TakeDamage rva=0x14a3930 -> ModifyDamage
    /// (native pass-through, `movaps xmm0, xmm1; ret` at 0x141489ee0) -> UDamageableComponent::ModifyDamage
    /// rva=0x1489be0 -> OnPostTakeDamage rva=0x1490ad0. With a game mode (TakeDamage disasm 0x14156e980..0x14156ef21):
    /// spawn protection (< spawn_protection_max_damage -> 0) and the friendly spawn window x spawn_damage_scale.
    /// Returns the damage as applied. source: the attacker's fighter index.
    pub fn take_damage(&mut self, fi: usize, amount: f64, source: Option<usize>, ty: i64) -> f64 {
        if self.fighters[fi].dead {
            return 0.0; // ShouldTakeDamage (vcall +0x698) false -> 0
        }
        let c = &self.spec.constants;
        let q = self.qf();
        let mut amount = q(amount);
        let age = q(self.now - self.fighters[fi].creation_time);
        if let Some(r) = &self.mode_rules {
            if age < r.spawn_protection_duration && amount < c.spawn_protection_max_damage {
                amount = 0.0;
            }
            if self.is_friendly(source, Some(fi)) && age < c.friendly_spawn_window {
                amount = q(amount * c.spawn_damage_scale);
            }
        }
        let applied = self.modify_damage(fi, amount, ty, source);
        self.post_take_damage(fi, applied, ty);
        applied
    }

    /// from UDamageableComponent::ModifyDamage rva=0x1489be0 (disasm 0x141489c22..0x141489e14 for the game-mode
    /// terms; Fall branch `if (EVar2 == Fall)`): see motion_system.gd _modify_damage for the full derivation.
    fn modify_damage(&self, fi: usize, amount: f64, ty: i64, source: Option<usize>) -> f64 {
        let c = &self.spec.constants;
        let q = self.qf();
        let f = &self.fighters[fi];
        let ch = &f.character;
        let mut d = amount;
        let friendly = self.is_friendly(source, Some(fi));
        if let Some(r) = &self.mode_rules {
            if r.damageable_spawn_protection_duration > 0.0 {
                if q(self.now - f.creation_time) < r.damageable_spawn_protection_duration {
                    d = q(d * c.spawn_damage_scale);
                }
                let factor = if r.b_disable_damage { 0.0 } else { q(r.damage_factor * if friendly { r.team_damage_factor } else { 1.0 }) };
                d = q(d * factor);
            }
        }
        if ty == DAMAGE_FALL {
            d = q(d * ch.received_fall_damage_modifier);
        } else {
            if ty == DAMAGE_RANGED {
                d = q(d * ch.received_ranged_damage_modifier);
            }
            d = q(d * ch.received_damage_modifier);
            if self.mode_rules.is_some() && friendly {
                d = q(d * ch.received_team_damage_modifier);
            }
        }
        let absorption = ch.received_damage_absorption;
        if d > 0.0 {
            d = maxf(q(d - absorption), 0.0);
        } else if d < 0.0 {
            d = crate::ue::minf(q(d + absorption), 0.0);
        }
        if d > 0.0 {
            d = maxf(d, c.min_damage);
            let max_damage = ch.received_damage_max;
            if max_damage > 0.0 && d > max_damage {
                d = max_damage;
            }
        }
        crate::ue::minf(d, f.health as f64)
    }

    /// from UDamageableComponent::OnPostTakeDamage rva=0x1490ad0 (disasm 0x141490be8..0x141490cd2):
    /// NewValue = int(float(StatValue) - Damage) (cvttss2si), < 0 -> 0; 0 with bHasLastChance on a Melee/Ranged hit
    /// -> max(LastChanceHealAmount, 1), bHasLastChance = false; SetStatValue.
    fn post_take_damage(&mut self, fi: usize, applied: f64, ty: i64) {
        let q = self.qf();
        let f = &mut self.fighters[fi];
        let mut nv = trunc_i(q(f.health as f64 - applied)); // cvtdq2ps / subss / cvttss2si
        if nv < 0 {
            nv = 0;
        }
        if nv == 0 && f.has_last_chance && (ty == DAMAGE_MELEE || ty == DAMAGE_RANGED) {
            nv = f.character.last_chance_heal_amount.max(1);
            f.has_last_chance = false;
        }
        let old = f.health;
        self.set_health(fi, nv);
        // SetStatValue(NewValue, bReplicate true) writes the ReplicatedHealth byte when the value changed
        if self.fighters[fi].health != old {
            if let Some(h) = self.net.clone() {
                h.stat_written(self, fi, true);
            }
        }
    }

    /// UHealthStatComponent::SetStatValue_Internal rva=0x14d5cf0: clamp, then OnDied when the value goes from > 0 to
    /// < 1; UStaminaStatComponent::OnCharacterDied rva=0x1508110: bIsRegenerable = false
    /// SetStatValue_Internal(v, bReplicate false) of the health stat (OnRep_ReplicatedHealth's path; OnDied on > 0 -> < 1)
    pub fn set_health_value(&mut self, fi: usize, nv: i64) {
        self.set_health(fi, nv);
    }

    fn set_health(&mut self, fi: usize, nv: i64) {
        let f = &mut self.fighters[fi];
        let old = f.health;
        let (mn, mx) = (f.health_stat.min_value, f.health_stat.max_value);
        f.health = if nv < mn { mn } else if nv < mx { nv } else { mx };
        if old > 0 && f.health < 1 && !f.dead {
            f.dead = true;
            f.stamina_regenerable = false;
            let name = f.name.clone();
            self.trace_event(&format!("{name} died"));
            self.emit_event(json!({"kind": "died", "who": name}));
        }
    }

    /// One step for this fighter = AMordhauCharacter::LODTick rva=0x154c390, in that function's order:
    ///  1. AAdvancedCharacter::LODTick rva=0x14887b0 -> UMotionSystemComponent::OnLODTick rva=0x14cb3e0: motion Tick
    ///     while !bIsDead (disasm 0x1414cb4a7)
    ///  2. the buffered parry: holding block inside a UFlinchMotion with a bIsParryHeld weapon sets bWantsBlock; while
    ///     bWantsBlock: bWantsBlock = !RequestParry(Regular, true) (disasm 0x14154c937..0x14154ca3c)
    ///  3. StopStaminaRegen while the motion bBlocksRegen (+0x88), then the stamina TickStat (vcall +0x410)
    pub fn fighter_tick(&mut self, fi: usize, dt: f64) {
        // AAdvancedCharacter::LODTick rva=0x14887b0: the turn caps (decomp 524-576), before the motion tick
        self.tick_turn_caps(fi, dt);
        if let Some(c) = self.fighters[fi].motion {
            if !self.fighters[fi].dead {
                self.motion_tick(fi, c, dt);
            }
        }
        // AMordhauCharacter::LODTick rva=0x154c390 (decomp 3669-3675): bWantsFire -> OnRequestFire (rangedmotion.rs)
        self.ranged_request_fire(fi);
        let f = &self.fighters[fi];
        let in_flinch = f.motion.map(|c| self.m(fi, c).is_flinch()).unwrap_or(false);
        if !f.wants_block && f.holding_block && in_flinch && f.weapon.as_ref().map(|w| w.b_is_parry_held).unwrap_or(false) {
            self.fighters[fi].wants_block = true;
        }
        if self.fighters[fi].wants_block {
            let ok = self.request_parry(fi, bt::REGULAR, true);
            self.fighters[fi].wants_block = !ok;
        }
        if let Some(c) = self.fighters[fi].motion {
            if self.m(fi, c).b_blocks_regen {
                self.stop_stamina_regen(fi, 0.0);
            }
        }
        self.tick_stamina_regen(fi);
    }

    /// The late tick: ULateTickComponent (ctor rva=0x14af270, TickGroup 4 = TG_PostPhysics) -> UMotionSystemComponent::
    /// OnLateTick rva=0x14cc120 -> UAttackMotion::OnLateTick_Implementation rva=0x1631f00: while an attack motion is
    /// current, the weapon prepares its tracers (vtable +0x7c0 PrepareForTracing)
    pub fn fighter_late_tick(&mut self, fi: usize) {
        if self.fighters[fi].dead {
            return;
        }
        // URangedDrawMotion::OnLateTick_Implementation rva=0x1664d10 (rangedmotion.rs)
        self.ranged_late_tick(fi);
        if self.cur_m(fi).map(|m| m.is_attack()).unwrap_or(false) {
            if let Some(h) = self.trace_host.clone() {
                h.prepare(self, fi);
                return;
            }
            let f = &mut self.fighters[fi];
            f.tracer.prepare_for_tracing();
        }
    }

    // ---- input ----
    /// AMordhauCharacter::RequestAttack rva=0x15644c0 -> UMotionSystemComponent::RequestAttack rva=0x14d0df0 ->
    /// AMordhauWeapon::RequestAttack_Implementation rva=0x163b700: CanPerformAttack false -> try the mirrored move;
    /// still false -> nothing; then Motion->ProcessAttack(Move, Angle)
    pub fn request_attack(&mut self, fi: usize, m: i64, angle: f64) {
        if self.fighters[fi].dead || self.fighters[fi].motion.is_none() {
            return;
        }
        let mut m = m;
        if !self.can_perform_attack(fi, m) {
            m = match m {
                mv::RIGHT_STRIKE => mv::LEFT_STRIKE,
                mv::LEFT_STRIKE => mv::RIGHT_STRIKE,
                mv::STAB => mv::ALT_STAB,
                mv::ALT_STAB => mv::STAB,
                _ => return,
            };
            if !self.can_perform_attack(fi, m) {
                return;
            }
        }
        let c = self.fighters[fi].motion.unwrap();
        self.motion_process_attack(fi, c, m, angle);
    }

    /// The preset attack inputs (RequestRightStrike 0x14d18d0 ... RequestLeftStab 0x14d1470) through
    /// UMotionSystemComponent::AdjustPresetAttackAngleRequest rva=0x14b2d80: FlipSide when bWantsFlipAttackSide
    /// (+0xd78, `cmp byte ptr [rbx + 0xd78], 0` at 0x1414b2e71). Angles: upper -57.5 / lower 60.0 (immediates).
    pub fn request_preset_attack(&mut self, fi: usize, m: i64, angle: f64) {
        let m = if self.fighters[fi].preset_flip { enums::flip_side(m) } else { m };
        self.request_attack(fi, m, angle);
    }

    /// from UMotionSystemComponent::CanInitiateMotion rva=0x14b4ba0: ask the current motion; if it refuses and
    /// bAttemptCancel with an attack current: ProcessFeint, then ask the (possibly new) motion again
    pub fn can_initiate_motion(&mut self, fi: usize, new_kind: &str, attempt_cancel: bool) -> bool {
        let Some(c) = self.fighters[fi].motion else { return false };
        let mut ok = self.motion_can_initiate(fi, c, new_kind);
        if !ok && attempt_cancel && self.m(fi, c).is_attack() {
            self.motion_process_feint(fi, c);
            let c2 = self.fighters[fi].motion.unwrap();
            ok = self.motion_can_initiate(fi, c2, new_kind);
        }
        ok
    }

    /// from UEquipmentSystemComponent::CheckCanEquipAlt rva=0x14b50e0 (disasm 0x1414b5129..0x1414b5194)
    pub fn check_can_equip_alt(&self, fi: usize, e: Option<&EquipmentDef>) -> bool {
        let Some(e) = e else { return false };
        if !e.b_has_alternate_mode {
            return false;
        }
        let f = &self.fighters[fi];
        if f.b_is_left_arm_disabled || f.b_is_right_arm_disabled {
            let side_disabled = if e.b_second_is_right_handed { f.b_is_right_arm_disabled } else { f.b_is_left_arm_disabled };
            if side_disabled || e.b_second_is_two_handed {
                return false;
            }
        }
        true
    }

    /// AMordhauCharacter::RequestFeint rva=0x15646b0: the current motion's ProcessFeint
    pub fn request_feint(&mut self, fi: usize) {
        if !self.fighters[fi].dead {
            let c = self.fighters[fi].motion.unwrap();
            self.motion_process_feint(fi, c);
        }
    }

    /// AMordhauCharacter::RequestParry rva=0x1564860 -> UMotionSystemComponent::RequestParry rva=0x14d1590 ->
    /// AMordhauWeapon::RequestBlock_Implementation rva=0x163b7b0 (+ AMordhauShield::RequestBlock_Implementation
    /// rva=0x15f16c0 for a shield-wall shield): ok = Motion->ProcessBlock; !ok, bAllowFTP and an attack current:
    /// ProcessFeint, then ProcessBlock again.
    pub fn request_parry(&mut self, fi: usize, b: i64, allow_ftp: bool) -> bool {
        let f = &self.fighters[fi];
        if f.dead || f.motion.is_none() || f.weapon.is_none() {
            return false;
        }
        let use_left = self.left_is_weapon(fi);
        let c = f.motion.unwrap();
        if use_left && f.left_hand.as_ref().unwrap().is_shield {
            if let Some(p) = self.m(fi, c).parry() {
                if p.b_is_shield_wall {
                    if p.stage == ps::PARRY && !p.b_requested_drop {
                        self.server_drop_parry(fi);
                        if let Some(c2) = self.fighters[fi].motion {
                            if let Some(p2) = self.mm(fi, c2).parry_mut() {
                                p2.b_requested_drop = true;
                            }
                        }
                    }
                    return false;
                }
            }
        }
        let f = &self.fighters[fi];
        let w = if use_left { f.left_weapon.as_ref().unwrap() } else { f.weapon.as_ref().unwrap() };
        // AMordhauWeapon::RequestBlock_Implementation rva=0x163b7b0 (decomp 5558): on a horse bCanBlockOnHorseback
        let place = if f.mount.is_some() { w.b_can_block_on_horseback } else { w.b_can_block_on_foot };
        if !w.b_can_block || !place {
            return false;
        }
        let ok = self.motion_process_block(fi, c, b);
        let c = self.fighters[fi].motion.unwrap();
        if ok || !allow_ftp || !self.m(fi, c).is_attack() {
            return ok;
        }
        self.motion_process_feint(fi, c);
        let c = self.fighters[fi].motion.unwrap();
        self.motion_process_block(fi, c, b)
    }

    /// AMordhauCharacter::BlockPressed rva=0x1531650: bIsHoldingBlock = true; bWantsBlock = !RequestParry(type, true)
    pub fn block_pressed(&mut self, fi: usize, b: i64) {
        self.fighters[fi].holding_block = true;
        let ok = self.request_parry(fi, b, true);
        self.fighters[fi].wants_block = !ok;
    }

    /// AMordhauCharacter::BlockReleased rva=0x15316a0: bWantsBlock = false, bIsHoldingBlock = false
    pub fn release_block(&mut self, fi: usize) {
        self.fighters[fi].wants_block = false;
        self.fighters[fi].holding_block = false;
    }

    /// UMotionSystemComponent::GetParryMotionClass rva=0x14be9b0: native UParryMotion; the right-hand weapon's
    /// profile ParryMotion if set; then the left-hand equipment's profile ParryMotion if set overrides it
    pub fn parry_motion_bp(&self, fi: usize) -> String {
        let f = &self.fighters[fi];
        let mut b = match &f.weapon_equip {
            Some(e) => self.spec.profile_motions(&e.weapon_animation_profile).parry_motion,
            None => String::new(),
        };
        if let Some(l) = &f.left_hand {
            let lp = self.spec.profile_motions(&l.weapon_animation_profile).parry_motion;
            if !lp.is_empty() {
                b = lp;
            }
        }
        b
    }

    /// UParryMotion::OnBegin_Implementation rva=0x1660f70 (disasm 0x14166112d..0x141661201): WeaponPtr (+0x550) =
    /// LeftHandEquipment if it IsA AMordhauWeapon, else RightHandEquipment
    pub fn parry_weapon(&self, fi: usize) -> Option<Rc<WeaponData>> {
        if self.left_is_weapon(fi) { self.fighters[fi].left_weapon.clone() } else { self.fighters[fi].weapon.clone() }
    }
    pub fn parry_weapon_equip(&self, fi: usize) -> Option<Rc<EquipmentDef>> {
        if self.left_is_weapon(fi) { self.fighters[fi].left_hand.clone() } else { self.fighters[fi].weapon_equip.clone() }
    }
    pub fn left_is_weapon(&self, fi: usize) -> bool {
        self.fighters[fi].left_hand.as_ref().map(|l| l.is_weapon).unwrap_or(false)
    }

    /// AMordhauCharacter::RequestToggleWeaponMode rva=0x1564a60 + AMordhauShield::OnRequestModeSwitch_Implementation
    /// rva=0x15e6d70 shield-wall branch; otherwise the weapon's mode switch (switch_mode: UEquipmentModeSwitchMotion,
    /// modeswitch.rs)
    pub fn toggle_weapon_mode(&mut self, fi: usize) {
        let f = &self.fighters[fi];
        if f.dead || f.motion.is_none() {
            return;
        }
        if self.left_is_weapon(fi) && f.left_hand.as_ref().unwrap().is_shield {
            let c = f.motion.unwrap();
            let lw = f.left_weapon.as_ref().unwrap();
            let idle = matches!(self.m(fi, c).k, MotionKind::Idle);
            if lw.b_can_block && f.left_hand.as_ref().unwrap().b_allow_shield_wall && idle {
                let mut ok = self.motion_process_block(fi, c, bt::SHIELD_WALL);
                let c2 = self.fighters[fi].motion.unwrap();
                if !ok && self.m(fi, c2).is_attack() {
                    self.motion_process_feint(fi, c2);
                    let c3 = self.fighters[fi].motion.unwrap();
                    ok = self.motion_process_block(fi, c3, bt::SHIELD_WALL);
                }
                if ok {
                    return;
                }
            }
        }
        self.switch_mode(fi);
    }

    // switch_mode / switch_mode_and_reattach: modeswitch.rs (fp-anim r3, UEquipmentModeSwitchMotion)

    /// AttackMotion/Parry helper for the movement layer: the current motion's GetMovementRestriction
    pub fn movement_restriction(&self, fi: usize) -> i64 {
        match self.fighters[fi].motion {
            Some(c) => self.motion_movement_restriction(fi, c),
            None => 0,
        }
    }

    /// convenience for rules: the current motion is an attack in this stage
    pub fn attack_stage(&self, fi: usize) -> Option<i64> {
        self.cur_m(fi).and_then(|m| m.attack()).map(|a| a.stage)
    }

    #[allow(dead_code)]
    pub(crate) fn is_type(&self, fi: usize, ty: i64) -> bool {
        self.cur_m(fi).and_then(|m| m.attack()).map(|a| a.ty == ty).unwrap_or(false)
    }
}

#[allow(unused_imports)]
use at as _at;

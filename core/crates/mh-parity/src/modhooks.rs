//! The NoChamber server mod's character script as a mod LAYER over mordhau-core's World (rust-parity r6).
//!
//! The mod's BP_MorhauCharacter (NoChamber/BP_MorhauCharacter, decoded bytecode
//! state/parity/mod_kismet/Mordhau/Content/Mordhau/Maps/WeaponRemovalMap_VersionA/NoChamber/BP_MorhauCharacter.txt)
//! overrides the native BlueprintImplementableEvent OnBlockedMelee (called from UParryMotion::CheckParry
//! rva=0x164ed40 after ReceiveBlock, decomp 1249-1272 = mordhau-core melee_hit.rs `apply_parry`). Two branches read
//! per-server switches that BP_ModificationActor_VersionA writes into the character CDO:
//!
//! 1. experimental parry (ubergraph 5757..6089), switched on by ModifyExperimentalParry (IsUsingExperimentalParry =
//!    true) from OnRep_ReplicatedExperimentalParry, duration by ModifyExperimentalParryDuration (ParryExtensionDuration
//!    = ReplicatedExpParryDuration; character CDO default 0.05, mod_json BP_MorhauCharacter):
//!        if IsUsingExperimentalParry and Cast<ParryMotion>(MotionSystemComponent.Motion):
//!            Motion.ParryUpTime = (GetGameTimeInSeconds() - Motion.StartTime) + ParryExtensionDuration
//!    so a parry that blocks stays up ParryExtensionDuration past the block (+ the native
//!    NonHeldParryExtensionAndRiposteWindowExtra the block sets) instead of its fixed ParryUpTime.
//! 2. chftp stamina (ubergraph 6090..7519), switched on by ModifyUseChftpStun (IsUsingChftpStun = true; the server
//!    sets it together with ReplicatedChftpStamCost = -2.0, actor ubergraph 9705..9917, and ModifyChftpStamCost writes
//!    that -2 into every parry motion's ChamberFTPExtraStaminaDrain, so the native chamber-feint-to-parry penalty of
//!    ReceiveBlock becomes a +2 refund):
//!        if IsUsingChftpStun and ComingFromMotion is BP_FeintedMotion_Comp or BP_FeintedMotion:
//!            (ExcludeEarlyReleaseCHFTP: skipped, default false, no replicated value in the demos)
//!            CalculateStandardStamDrain(Attacker) -> ExpectedStam = StaminaAtParryStart_0 -
//!                FClamp(AttackStamDrain - DefendStamNegation, StamClamp.X, StamClamp.Y), IsStab
//!                (IsStab = false unless ShouldExcludeChftpStabs; AttackStamDrain = ModAc.WeaponInfos[attacker class]
//!                 [move].Z = the attack's installed StaminaDrain, ModifyWeaponStats)
//!            if ExpectedStam + 2 == float(Stamina):          (only the native +2 chftp refund happened)
//!                IsStab ? OffsetStamina(-2) : OffsetStamina(FTrunc(-(ChftpStamCost + 2)))
//!                BP_ParryMotion.BlockStaminaRecover = ChftpStamForAttacker
//!                Stamina == 0 -> AssignNetMotionSimple(29 = stun, ...) (+ OffsetStamina(30) when DoesStunDisarm)
//!    StaminaAtParryStart_0 = float(Stamina) at the parry motion's BP OnBegin (BP_*ParryMotion_Comp ubergraph 663..722).
//!    ChftpStamCost = 15.0 (character CDO; the server's CustomChftpStunStamCost ini override is not replicated:
//!    UNCONFIRMED per server). Not ported here: the BlockStaminaRecover instance write (mordhau-core reads the class
//!    default; ChftpStamForAttacker = ReplicatedChftpAttackerRewardStam = 0 on every server) and the stun at 0 stamina.
//!
//! Applied after each World step (the BP event runs inside the step; both writes are only read by later ticks: the
//! parry's stop-holding test q(q(ParryUpTime + Start) + NonHeldExt) < now is false at the block tick for any
//! ParryUpTime >= now - Start, and the chftp offset is a stamina write). A block = a "parry" event of the step whose
//! victim's current motion is still that parry (the BP's Cast<ParryMotion>(Motion) fails after a queued riposte).

use mordhau_core::combat::{MotionKind, World};
use serde_json::Value;
use std::collections::HashMap;

/// The per-server switches (mod_overlay.py writes them into the layer as "mod_hooks")
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModHooks {
    /// ParryExtensionDuration when IsUsingExperimentalParry
    pub exp_parry: Option<f64>,
    /// ChftpStamCost when IsUsingChftpStun
    pub chftp_stam_cost: Option<f64>,
    /// ShouldExcludeChftpStabs (ModifyExcludeStabChftpStam)
    pub chftp_exclude_stabs: bool,
}

impl ModHooks {
    pub fn from_layer(layer: &Value) -> ModHooks {
        let h = &layer["mod_hooks"];
        ModHooks {
            exp_parry: h["experimental_parry_duration"].as_f64(),
            chftp_stam_cost: h["chftp_stam_cost"].as_f64(),
            chftp_exclude_stabs: h["chftp_exclude_stabs"].as_bool().unwrap_or(false),
        }
    }
    pub fn any(&self) -> bool {
        self.exp_parry.is_some() || self.chftp_stam_cost.is_some()
    }
}

/// Per-run state of the hooks
#[derive(Default)]
pub struct HookState {
    hits_seen: usize,
    /// fighter -> (parry motion id, StaminaAtParryStart_0)
    parry_start: HashMap<usize, (u32, i64)>,
}

impl HookState {
    pub fn after_step(&mut self, w: &mut World, h: &ModHooks) {
        // StaminaAtParryStart_0: the parry motion's OnBegin
        for fi in 0..w.fighters.len() {
            let Some(id) = w.fighters[fi].motion else { continue };
            if !w.m(fi, id).is_parry() {
                continue;
            }
            if self.parry_start.get(&fi).map(|p| p.0) != Some(id.0) {
                let s = w.stamina_byte(fi);
                self.parry_start.insert(fi, (id.0, s));
            }
        }
        let new: Vec<(String, String)> = w.hits[self.hits_seen..]
            .iter()
            .filter(|e| e.get("kind").and_then(|k| k.as_str()) == Some("parry"))
            .map(|e| (e["victim"].as_str().unwrap_or("").to_string(), e["attacker"].as_str().unwrap_or("").to_string()))
            .collect();
        self.hits_seen = w.hits.len();
        for (vn, an) in new {
            let (Some(v), Some(a)) = (w.fighter_index(&vn), w.fighter_index(&an)) else { continue };
            self.on_blocked_melee(w, h, v, a);
        }
    }

    /// BP_MorhauCharacter OnBlockedMelee (ubergraph 5757..7519) on the parrier `v`, attacker `a`
    fn on_blocked_melee(&mut self, w: &mut World, h: &ModHooks, v: usize, a: usize) {
        let q = w.qf();
        let Some(id) = w.fighters[v].motion else { return };
        // 5800 Cast<ParryMotion>(MotionSystemComponent.Motion)
        if let Some(ext) = h.exp_parry {
            let now = w.now;
            let m = w.mm(v, id);
            let start = m.start_time;
            if let MotionKind::Parry(p) = &mut m.k {
                // 5897..6040 ParryUpTime = (GetGameTimeInSeconds - StartTime) + ParryExtensionDuration (f32 BP math)
                p.parry_up_time = q(q(now - start) + ext);
            }
        }
        let Some(cost) = h.chftp_stam_cost else { return };
        // 6100..6287 ComingFromMotion class == BP_FeintedMotion_Comp || BP_FeintedMotion
        let from_feint = w.m(v, id).coming_from.map(|c| matches!(w.m(v, c).k, MotionKind::Feinted(_))).unwrap_or(false);
        if !from_feint {
            return;
        }
        // CalculateStandardStamDrain: the attacker's LastAttackMotion
        let Some(la) = w.fighters[a].last_attack_motion else { return };
        let Some(at) = w.m(a, la).attack() else { return };
        let (drain, mv) = (at.ai.stamina_drain, at.mv);
        let Some(wd) = w.parry_weapon(v) else { return };
        let (neg, clamp) = (wd.block_stamina_negation, wd.block_stamina_clamp);
        let x = q(drain - neg);
        let c = x.max(clamp.xf()).min(clamp.yf()); // FClamp(x, X, Y)
        let start_stam = self.parry_start.get(&v).filter(|p| p.0 == id.0).map(|p| p.1).unwrap_or(w.stamina_byte(v));
        let expected = q(start_stam as f64 - c);
        let is_stab = h.chftp_exclude_stabs && (mv == 2 || mv == 3);
        // 6939..7056 ExpectedStam + 2.0 == Conv_ByteToFloat(Stamina)
        if q(expected + 2.0) != w.stamina_byte(v) as f64 {
            return;
        }
        if is_stab {
            w.offset_stamina(v, -2); // 7080
        } else {
            w.offset_stamina(v, q(-q(cost + 2.0)).trunc() as i64); // 7097..7218 FTrunc(-(ChftpStamCost + 2))
        }
    }
}

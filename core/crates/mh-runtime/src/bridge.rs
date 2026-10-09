//! bridge.rs - the sim's events and state into rust-assets' mh-ui (UMG HUD), mh-fx (Cascade particles) and mh-audio
//! (SoundCues) plugins (r4 item 6; interfaces agreed with rust-assets r3).
//!   - every combat event the sim drains (mordhau-core emit_event: hit / parry / active_parry / chamber / clash /
//!     was_blocked ...) -> FxRequest (CombatFx::for_event; armoured = BloodMetalHitEffect choice of
//!     OnTookDamage_Implementation rva=0x155be10) + the weapon sounds the exe plays (onhit.rs: OnHit rva=0x1631430 on
//!     the attacker's weapon with HitLocation / ArmorTier / SurfaceType / IsStab / IsSourceViewTarget cue params and
//!     the damage-scaled EnvironmentHitSound; OnBlocked rva=0x16306d0 on the defender's weapon with Reason;
//!     OnWasBlocked rva=0x16340a0 on the attacker's) at the event's position (the attacker's current trace end, UE cm
//!     -> Bevy m; the exe's ImpactPoint / trace-corrected sound point is UNCONFIRMED here, fidelity_audit.md)
//!   - the player fighter's health / stamina -> HudVitals (BP_StatusBar bytes)
//!   - the camera -> AudioListener
//!   - character sounds (mh_audio::game, rust-assets r8; events from mh-sim's stream, rust-combat r6): "release" ->
//!     Trigger::Release (whoosh + ReleaseFoley, UAttackMotion::OnTick rva 0x16328c0), "motion" -> MotionBegin (armour
//!     foley) + AttackYell on melee Attack begin (stand-in: the Blueprint caller of PlayAttackYell is UNCONFIRMED),
//!     "crouch_start" / "crouch_end", "foot_landed" -> FootLanded (surface = the floor body's physical material
//!     SurfaceType, SimBackend::floor_surface), "fall_damage" -> FallDamage, "hit" -> Hurt (victim), "died" -> Death
//!   - a loaded map's placed sounds / particle systems -> mh_audio::MapSounds / mh_fx::MapEffects
//! FX / sound choice and playback rules are the plugins' (rust-assets); this module only routes.

use crate::sim::Sim;
use bevy::prelude::*;
use std::collections::HashMap;

#[derive(Resource, Default)]
pub struct BridgeState {
    pub fx: Option<mh_fx::CombatFx>,
    pub sounds: HashMap<String, mh_audio::WeaponSounds>,
    /// weapon Blueprint -> its OnHit / OnBlocked / OnWasBlocked cue fields (onhit.rs)
    pub hit_data: HashMap<String, crate::onhit::WeaponHitData>,
    pub events: usize,
    pub fx_sent: usize,
    pub cues_sent: usize,
    /// SC_HitIndicator plays (the attacker's hit indicator, fidelity-audit r8)
    pub hit_markers: usize,
    pub last_events: Vec<serde_json::Value>,
    pub chars: Option<mh_audio::game::CharacterSounds>,
    pub wooshes: HashMap<String, mh_audio::game::WeaponWoosh>,
    pub voices: HashMap<String, mh_audio::sources::VoicePack>,
    /// trigger kind -> cues sent (evidence)
    pub triggers: std::collections::BTreeMap<String, usize>,
    /// the map whose MapSounds / MapEffects are in
    pub map_ambience: String,
    pub map_sounds: usize,
    pub map_effects: usize,
    pub map_spawners: usize,
    pub map_volumes: usize,
}

pub struct BridgePlugin;

impl Plugin for BridgePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BridgeState>().init_resource::<crate::weapon::blood::WeaponBlood>().init_resource::<TrailState>().init_resource::<crate::shake::CameraShakes>().insert_resource(crate::usersettings::UserSettings::load()).add_systems(Update, (prewarm_combat, forward_events, vitals, listener, map_ambience))
            .add_systems(PostUpdate, crate::weapon::blood::update);
    }
}

/// every package path ("Mordhau/Content/...") quoted in a value's Debug text (the sound tables are plain structs of
/// cue paths)
fn quoted_paths(dbg: &str) -> Vec<String> {
    dbg.split('"').filter(|t| t.starts_with("Mordhau/Content/") || t.starts_with("Engine/Content/")).map(|t| t.to_string()).collect()
}

/// Loading glue (first-person r1, smoothness): a fighter's weapon tables (hit / block / woosh sounds) and the shared
/// character / FX tables are read when the fighter appears, and every cue they name is loaded and decoded
/// (mh_audio::PrewarmCue), so the first hit of a fight does not stall the frame it lands on (it measured 262 ms).
fn prewarm_combat(
    sim: NonSend<Sim>,
    mut st: ResMut<BridgeState>,
    src: Res<crate::source::Source>,
    mut warm: MessageWriter<mh_audio::PrewarmCue>,
    mut warm_fx: MessageWriter<mh_fx::FxPrewarm>,
    mut seen: Local<std::collections::HashSet<String>>,
) {
    let Some(vfs) = &src.vfs else { return };
    let views = sim.0.fighters();
    if views.is_empty() {
        return;
    }
    let rd = mh_pak::Reader::new(vfs.clone());
    let mut paths: Vec<String> = Vec::new();
    if st.fx.is_none() {
        let f = mh_fx::CombatFx::read(&rd);
        let mut systems: Vec<String> = vec![f.block.clone(), f.hit_cancel.clone(), f.slide.clone(), f.armor_hit.clone(), f.flesh_hit.clone()];
        systems.extend(f.impact_by_surface.iter().cloned());
        systems.sort();
        systems.dedup();
        for sys in systems.into_iter().filter(|x| !x.is_empty()) {
            warm_fx.write(mh_fx::FxPrewarm(sys));
        }
        st.fx = Some(f);
    }
    if st.chars.is_none() {
        let c = mh_audio::game::CharacterSounds::read(&rd);
        paths.extend(quoted_paths(&format!("{c:?}")));
        st.chars = Some(c);
    }
    for v in &views {
        let Some(w) = sim.0.weapon_path(v.id) else { continue };
        if w.is_empty() || !seen.insert(w.clone()) {
            continue;
        }
        let hd = st.hit_data.entry(w.clone()).or_insert_with(|| crate::onhit::WeaponHitData::read(&rd, &w)).clone();
        paths.extend(quoted_paths(&format!("{hd:?}")));
        let ws = st.sounds.entry(w.clone()).or_insert_with(|| mh_audio::WeaponSounds::read(&rd, &w)).clone();
        paths.extend(quoted_paths(&format!("{ws:?}")));
        let wo = st.wooshes.entry(w.clone()).or_insert_with(|| mh_audio::game::WeaponWoosh::read(&rd, &w)).clone();
        paths.extend(quoted_paths(&format!("{wo:?}")));
    }
    paths.sort();
    paths.dedup();
    for p in paths {
        warm.write(mh_audio::PrewarmCue(p));
    }
}

fn ue_to_m(p: &serde_json::Value) -> Option<Vec3> {
    let a = p.as_array()?;
    let g = |i: usize| a.get(i).and_then(|x| x.as_f64()).map(|x| x as f32);
    Some(Vec3::new(g(0)?, g(2)?, g(1)?) * 0.01)
}

/// the character sound triggers of a sim event, with the fighter name each belongs to
fn trigger_of(ev: &serde_json::Value) -> Vec<(String, mh_audio::game::Trigger)> {
    use mh_audio::game::Trigger;
    let s = |k: &str| ev.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
    let f = |k: &str| ev.get(k).and_then(|x| x.as_f64()).unwrap_or(0.0);
    let b = |k: &str| ev.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
    let v = |k: &str| ev.get(k).and_then(ue_to_m).unwrap_or(Vec3::ZERO);
    match s("kind").as_str() {
        // the attack yell: UAttackMotion::OnTick_Implementation rva=0x16328c0 (decomp UAttackMotion.cpp 2580-2604) plays
        // UCharacterVoiceComponent::PlayAttackYell (rva of the caller's callee in UCharacterVoiceComponent.cpp 747) once,
        // in Stage 0/1, when the motion time passes max(-LagInduction, 0) + WindupEnd + PlayAttackYellTimeReleaseOffset
        // (UAttackMotion ctor -0.05, UKickMotion ctor 0.0); no random chance. An "attack_yell" event from the sim is that
        // moment; without it the yell goes with "release" (WindupEnd: 0.05 s late for weapon attacks, UNCONFIRMED
        // until mh-sim emits attack_yell; it used to fire at motion begin, a whole windup early)
        "attack_yell" => vec![(s("who"), Trigger::AttackYell)],
        "release" => {
            let mut out = vec![(s("who"), Trigger::Release { trace_start: v("trace_start"), trace_end: v("trace_end"), stab: b("stab"), release_duration: f("release_duration") })];
            if !b("yell_emitted") {
                out.push((s("who"), Trigger::AttackYell));
            }
            out
        }
        "motion" => vec![(s("who"), Trigger::MotionBegin { kind: s("motion") })],
        "crouch_start" => vec![(s("who"), Trigger::CrouchStart)],
        "crouch_end" => vec![(s("who"), Trigger::CrouchEnd)],
        "fall_damage" => vec![(s("who"), Trigger::FallDamage)],
        // the event's surface is filled by the caller (floor_component -> physical material)
        "foot_landed" => vec![(s("who"), Trigger::FootLanded { foot: f("foot") as usize, surface: f("surface") as usize, speed_cm_s: f("speed"), crouched: b("crouched"), foot_pos: v("foot_pos") })],
        // the hurt yell on damage that does not kill: AMordhauCharacter::OnTookDamage_Implementation rva=0x155be10 (decomp
        // AMordhauCharacter.cpp 5326-5331: `if bWillKill {bleed-out} else if Type != Fire -> PlayHurtYell rva=0x1499160`);
        // the death yell comes with "died"
        "hit" if f("applied") > 0.0 && ev.get("health").and_then(|h| h.as_f64()).is_none_or(|h| h > 0.0) => vec![(s("victim"), Trigger::Hurt)],
        "died" => vec![(s("who"), Trigger::Death)],
        _ => vec![],
    }
}

fn trigger_name(t: &mh_audio::game::Trigger) -> &'static str {
    use mh_audio::game::Trigger as T;
    match t {
        T::Release { .. } => "release",
        T::MotionBegin { .. } => "motion",
        T::CrouchStart => "crouch_start",
        T::CrouchEnd => "crouch_end",
        T::FootLanded { .. } => "foot_landed",
        T::AttackYell => "attack_yell",
        T::Hurt => "hurt",
        T::Death => "death",
        T::Dodge => "dodge",
        T::FallDamage => "fall_damage",
    }
}

#[allow(clippy::too_many_arguments)]
pub fn forward_events(
    mut sim: NonSendMut<Sim>,
    mut st: ResMut<BridgeState>,
    src: Res<crate::source::Source>,
    mut fx: MessageWriter<mh_fx::FxRequest>,
    mut cues: MessageWriter<mh_audio::PlayCue>,
    sl: Res<crate::sim::SimLevel>,
    sel: Option<Res<crate::loadout::Selected>>,
    pc: Option<Res<crate::input::PlayerControl>>,
    mut shakes: ResMut<crate::shake::CameraShakes>,
    mut shake_data: Local<crate::shake::ShakeData>,
    us: Option<Res<crate::usersettings::UserSettings>>,
    mut trails: ResMut<TrailState>,
    mut trail_out: MessageWriter<mh_fx::trail::TrailEvent>,
    mut hud: MessageWriter<mh_ui::HudEvent>,
    mut blood: ResMut<crate::weapon::blood::WeaponBlood>,
) {
    let evs = sim.0.drain_events();
    let Some(vfs) = &src.vfs else { return };
    let rd = mh_pak::Reader::new(vfs.clone());
    // weapon trails: every frame (the sockets), with this frame's events
    trail_events(&evs, &mut sim, &mut trails, &rd, &mut trail_out);
    if evs.is_empty() {
        return;
    }
    if st.fx.is_none() {
        st.fx = Some(mh_fx::CombatFx::read(&rd));
    }
    if st.chars.is_none() {
        st.chars = Some(mh_audio::game::CharacterSounds::read(&rd));
    }
    let views = sim.0.fighters();
    let me = pc.as_ref().map(|p| p.id).unwrap_or(0);
    let my_team = views.iter().find(|v| v.id == me).and_then(|v| v.team);
    for mut ev in evs {
        blood.on_event(&ev, &rd, us.as_deref().is_none_or(|u| u.show_blood()));
        // footsteps: the floor body's EPhysicalSurface
        if ev.get("kind").and_then(|k| k.as_str()) == Some("foot_landed") {
            // mh-sim fills "surface" through surface_of (mh-level surface_at); the physical-material lookup only when
            // it could not
            let have = ev.get("surface").and_then(|s| s.as_u64()).unwrap_or(0) != 0;
            if let (false, Some(c)) = (have, ev.get("floor_component").and_then(|c| c.as_u64())) {
                ev["surface"] = serde_json::json!(sim.0.floor_surface(c as u32));
            }
        }
        for (who, trig) in trigger_of(&ev) {
            let Some(id) = sim.0.id_of(&who) else { continue };
            let Some(v) = views.iter().find(|v| v.id == id) else { continue };
            let weapon = sim.0.weapon_path(id).unwrap_or_default();
            let woosh = st.wooshes.entry(weapon.clone()).or_insert_with(|| mh_audio::game::WeaponWoosh::read(&rd, &weapon)).clone();
            let prof = sl.profiles.get(&id).copied();
            let voice_bp = sel.as_ref().and_then(|s| s.gear.get(&prof.unwrap_or(s.player))).map(|g| g.voice.clone()).unwrap_or_default();
            let voice = if voice_bp.is_empty() {
                None
            } else {
                Some(st.voices.entry(voice_bp.clone()).or_insert_with(|| mh_audio::sources::VoicePack::read(&rd, &voice_bp)).clone())
            };
            // the torso's armour tier (GetArmorTierForBone; mordhau-core damage::armor_tier)
            let armor_tier = match (sim.0.combat(), sim.0.fighter_index(id)) {
                (Some(w), Some(fi)) => w.fighters.get(fi).map(|f| mordhau_core::combat::damage::armor_tier(f, "Spine1")).unwrap_or(0),
                _ => 0,
            };
            let speaker = mh_audio::game::Speaker {
                pos: Vec3::new(v.loc[0], v.loc[2], v.loc[1]) * 0.01,
                weapon: woosh,
                voice,
                armor_tier,
                view_target: id == me,
                friendly: id != me && v.team.is_some() && v.team == my_team && v.team != Some(255),
            };
            let out = match st.chars.as_ref() {
                Some(chars) => mh_audio::game::cues(chars, &speaker, &trig),
                None => Vec::new(),
            };
            let n = out.len();
            for c in out {
                cues.write(c);
            }
            st.cues_sent += n;
            *st.triggers.entry(trigger_name(&trig).to_string()).or_default() += n;
        }
        st.events += 1;
        // camera shakes of the view target (shake.rs: PlayHitShake / OnRep_NetBlock / UBlockedMotion / UFlinchMotion)
        let cam1p = pc.as_ref().map(|p| (p.pitch, p.yaw)).unwrap_or((0.0, 0.0));
        shakes.headbob = us.as_ref().map(|u| u.headbob);
        shakes.on_event(&mut shake_data, &rd, sim.0.as_ref(), &ev, me, cam1p);
        // the attacker's hit indicator (fidelity-audit r8): AMordhauCharacter::TakeDamage rva=0x156e980 (decomp
        // AMordhauCharacter.cpp 6434-6475) sends the instigating player ClientReceiveScoreNoState(Reason Damage 4 / TeamDamage
        // 5, ReasonParam head 1 / leg 2 / else 0) when the damage is > 0 and not self-inflicted; BP_MordhauPlayerController
        // ClientReceiveScoreBP (state/ui_kismet/BP_MordhauPlayerController.txt @21615-20862): Damage -> ScoreFeed.AddDamage,
        // Crosshair.ShowHitMarker(param), SpawnSound2D(SC_HitIndicator) with int parameter "Param" = param. The HUD
        // calls are rust-ui's (mh_ui host hit_marker); the sound is played here
        if ev.get("kind").and_then(|k| k.as_str()) == Some("hit") && ev.get("friendly").and_then(|f| f.as_bool()) != Some(true) {
            let att = ev.get("attacker").and_then(|a| a.as_str()).and_then(|n| sim.0.id_of(n));
            let vic = ev.get("victim").and_then(|a| a.as_str()).and_then(|n| sim.0.id_of(n));
            let applied = ev.get("applied").and_then(|x| x.as_f64()).or_else(|| ev.get("damage").and_then(|x| x.as_f64())).unwrap_or(0.0);
            if att == Some(me) && vic != Some(me) && applied > 0.0 {
                let param = match ev.get("hit_location").and_then(|x| x.as_i64()) {
                    Some(0) => 1.0,
                    Some(2) => 2.0,
                    _ => 0.0,
                };
                let mut params = std::collections::HashMap::new();
                params.insert("Param".to_string(), param);
                cues.write(mh_audio::PlayCue { cue: "Mordhau/Content/Mordhau/Audio/Cues/UI/SC_HitIndicator".into(), params, two_d: true, ..default() });
                st.hit_markers += 1;
                // ScoreFeed.AddDamage + Crosshair.ShowHitMarker (rust-ui's HUD host)
                hud.write(mh_ui::HudEvent::DealtDamage { amount: applied.round() as i64, param: param as i64 });
            }
            // the victim's damage indicator: OnTookDamage_Implementation rva=0x155be10 (decomp 5338-5360) ->
            // AMordhauCameraManager::DoHitFlash rva=0x14f6600 (directional: melee / ranged damage from another actor;
            // HitFlashIgnoreUntilTime is never raised in the duel path) -> BP_MordhauCameraManager OnHitFlash ->
            // Crosshair.TriggerDamageIndicator(HitFlashDegrees). UpdateHitFlashDegrees rva=0x151e260 (decomp 613-690):
            // Atan2(Y, X) x 57.295776 of (source root location - camera location) unrotated by the camera yaw. Camera
            // location taken as the view target's actor location (UNCONFIRMED: horizontal only, the 1P camera sits above it)
            if vic == Some(me) && att.is_some() && att != Some(me) && applied > 0.0 {
                let src = att.and_then(|a| views.iter().find(|v| v.id == a)).map(|v| v.loc);
                let cam = views.iter().find(|v| v.id == me).map(|v| v.loc);
                if let (Some(s), Some(c)) = (src, cam) {
                    let yaw = pc.as_ref().map(|p| p.yaw).unwrap_or(0.0).to_radians();
                    let (dx, dy) = (s[0] - c[0], s[1] - c[1]);
                    // FRotator::UnrotateVector by (0, yaw, 0)
                    let (lx, ly) = (dx * yaw.cos() + dy * yaw.sin(), -dx * yaw.sin() + dy * yaw.cos());
                    hud.write(mh_ui::HudEvent::TookDamage { degrees: (ly.atan2(lx) * 57.295776) as f64 });
                }
            }
        }
        // the hit's ImpactPoint (the WorldLocation OnHit_Implementation rva=0x1631430 gets from OnTookDamage's Point) when
        // the sim reports it, else the attacker's current trace end. UNCONFIRMED refinement not ported: the sound point
        // is pulled 20 cm back along a channel-4 trace from the attacker's Spine1 (decomp AMordhauWeapon.cpp 507-541)
        let pos = ev.get("impact").or_else(|| ev.get("pos_ue")).and_then(ue_to_m).unwrap_or(Vec3::ZERO);
        let kind = ev.get("kind").and_then(|k| k.as_str()).unwrap_or("").to_string();
        let name_id = |k: &str| ev.get(k).and_then(|a| a.as_str()).and_then(|n| sim.0.id_of(n));
        // the blood HitEffect class: BloodMetalHitEffect for armour tier >= 2 unless the victim is the view target
        // (AMordhauCharacter::OnTookDamage_Implementation rva=0x155be10; onhit::metal_hit_effect)
        let tier = ev.get("tier").and_then(|t| t.as_i64()).unwrap_or(0);
        let armoured = kind == "hit" && crate::onhit::metal_hit_effect(tier, name_id("victim") == Some(me));
        // blood only with m.Gore != 0 (AAdvancedCharacter::PlayHitEffectParticle rva=0x1498f40 ->
        // UMordhauGameUserSettings::ShouldShowBlood rva=0x15a8510; the player's Gore setting). Its ConsumeBudget(0.25 s,
        // 2000 cm) rate limit is not ported (UNCONFIRMED budget semantics)
        let blood_off = kind == "hit" && us.as_deref().is_some_and(|u| !u.show_blood());
        let systems: Vec<String> = st.fx.as_ref().filter(|_| !blood_off).map(|f| f.for_event_all(&ev, armoured).into_iter().map(String::from).collect()).unwrap_or_default();
        for sys in systems {
            // no point on the event (was_blocked carries none yet: FBlockResult.Point UNCONFIRMED in mh-sim) -> skip
            if pos == Vec3::ZERO {
                continue;
            }
            // orientation: a character hit's HitEffect = MakeFromXZ(-LastObservedTraceDirection, Up)
            // (AAdvancedCharacter::OnCosmeticHit_Implementation rva=0x148c130, rust-assets r9); BlockParticles spawn with
            // FRotator::ZeroRotator (OnBlocked_Implementation rva=0x16306d0) = the UE +X axis (Bevy +X)
            let dir = if kind == "hit" {
                mh_fx::CombatFx::hit_dir(ev.get("trace_dir").and_then(|d| ue_to_m(d)).map(|v| v * 100.0).unwrap_or(Vec3::ZERO))
            } else {
                Vec3::X
            };
            fx.write(mh_fx::FxRequest { system: sys, pos, dir });
            st.fx_sent += 1;
        }
        // a world hit (no character): OnHit_Implementation rva=0x1631430 (decomp AMordhauWeapon.cpp 761-776) spawns
        // ImpactParticlesBySurface[SurfaceType] at the hit point, rotation MakeFromXZ(-LastObservedTraceDirection, Up)
        if kind == "world_hit" {
            let sf = ev.get("surface").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
            if let Some(sys) = st.fx.as_ref().and_then(|f| f.impact_by_surface.get(sf)).filter(|p| !p.is_empty()).cloned() {
                let dir = mh_fx::CombatFx::hit_dir(ev.get("trace_dir").and_then(|d| ue_to_m(d)).map(|v| v * 100.0).unwrap_or(Vec3::ZERO));
                fx.write(mh_fx::FxRequest { system: sys, pos, dir });
                st.fx_sent += 1;
            }
        }
        // weapon sounds as the exe picks them (onhit.rs: OnHit / OnBlocked / OnWasBlocked)
        let ranged = ev.get("ranged").and_then(|r| r.as_bool()).unwrap_or(false) || ev.get("horse").is_some();
        let b = |k: &str| ev.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
        let i = |k: &str, d: i64| ev.get(k).and_then(|x| x.as_i64()).unwrap_or(d);
        let (sound_weapon, hit_cues): (Option<String>, Box<dyn Fn(&crate::onhit::WeaponHitData) -> Vec<crate::onhit::HitCue>>) = match kind.as_str() {
            "hit" if !ranged => {
                // the attacker's weapon OnHit(victim, Move, Bone, Point, Tier, SurfaceType) from OnTookDamage
                let aid = name_id("attacker");
                let ai = match (sim.0.combat(), aid.and_then(|a| sim.0.fighter_index(a))) {
                    (Some(w), Some(fi)) => w.fighters.get(fi).and_then(|f| f.weapon.as_ref()).map(|wd| {
                        let mv = i("move", 0);
                        // OnHit_Implementation: Move < 2 StrikeAttack (+0x13e0), 2/3 StabAttack (+0xf40), else KickAttack (+0x1630)
                        let a = if mv < 2 { &wd.strike } else if mv <= 3 { &wd.stab } else { &wd.kick };
                        crate::onhit::AttackNums { damage: a.damage.clone(), head_bonus: a.head_bonus.clone(), stop_on_hit: a.b_stop_on_hit }
                    }),
                    _ => None,
                };
                let (mv, hl, sf, svt) = (i("move", 0), i("hit_location", 1), i("surface", 1), aid == Some(me));
                (aid.and_then(|a| sim.0.weapon_path(a)), Box::new(move |w| crate::onhit::on_hit(w, mv, hl, tier, sf, ai.as_ref(), svt)))
            }
            // the defender's OnRep_NetBlock -> its weapon's OnBlocked (EBlockedReason Parry 0 / Chamber 1 / Clash 3)
            "parry" | "active_parry" | "chamber" | "clash" => {
                let reason = match kind.as_str() {
                    "chamber" => crate::onhit::BR_CHAMBER,
                    "clash" => crate::onhit::BR_CLASH,
                    _ => crate::onhit::BR_PARRY,
                };
                let did = name_id("victim");
                let vt = did == Some(me);
                let disarm = b("disarm");
                (did.and_then(|d| sim.0.weapon_path(d)), Box::new(move |w| crate::onhit::on_blocked(w, reason, disarm, vt)))
            }
            // the attacker's UBlockedMotion -> OnWasBlocked (FBlockResult.bIsCancel: "cancel" when the sim reports it)
            "was_blocked" => {
                let (reason, cancel) = (i("reason", 0), b("cancel"));
                (name_id("who").and_then(|d| sim.0.weapon_path(d)), Box::new(move |w| crate::onhit::on_was_blocked(w, reason, cancel)))
            }
            // OnWasBlocked with bRequiresSelfBlockEvent: OnHit(null, Move, None, point, Tier 0, Result.Surface) on the
            // attacker's weapon (decomp AMordhauWeapon.cpp 1772): Bone None -> HitLocation 1
            "world_hit" => {
                let (mv, sf) = (i("move", 0), i("surface", 0));
                let aid = name_id("who");
                let (stop_on_hit, svt) = match (sim.0.combat(), aid.and_then(|a| sim.0.fighter_index(a))) {
                    (Some(w), Some(fi)) => w.fighters.get(fi).map(|f| {
                        // OnHit reads the weapon's Strike/Stab/Kick AttackInfo, not the now-blocked motion's copy.
                        let stop = f.weapon.as_ref().is_some_and(|wd| {
                            let a = if mv < 2 { &wd.strike } else if mv <= 3 { &wd.stab } else { &wd.kick };
                            a.b_stop_on_hit
                        });
                        (stop, f.is_view_target)
                    }).unwrap_or((false, false)),
                    _ => (false, false),
                };
                (aid.and_then(|a| sim.0.weapon_path(a)), Box::new(move |w| crate::onhit::on_world_hit(w, mv, sf, stop_on_hit, svt)))
            }
            _ => (None, Box::new(|_| Vec::new())),
        };
        // the sound's owner: the attacker (hit / was_blocked / world_hit) or the defender (blocks)
        let owner = match kind.as_str() {
            "hit" => name_id("attacker"),
            "parry" | "active_parry" | "chamber" | "clash" => name_id("victim"),
            _ => name_id("who"),
        };
        let alt = match (sim.0.combat(), owner.and_then(|o| sim.0.fighter_index(o))) {
            (Some(w), Some(fi)) => w.fighters.get(fi).is_some_and(|f| f.alternate_mode),
            _ => false,
        };
        if let Some(wp) = sound_weapon {
            let hd = st.hit_data.entry(wp.clone()).or_insert_with(|| crate::onhit::WeaponHitData::read(&rd, &wp)).clone();
            let hd = if alt { hd.switched() } else { hd };
            for c in hit_cues(&hd) {
                let params = c.param_map();
                cues.write(mh_audio::PlayCue { cue: c.cue, pos, params, volume: c.volume, pitch: c.pitch, ..default() });
                st.cues_sent += 1;
                *st.triggers.entry(format!("weapon_{kind}")).or_default() += 1;
            }
        } else if ranged {
            // ranged / horse hits: the projectile's FleshImpactSound path (AMordhauProjectile.cpp 3504 / 6949) is not
            // ported yet (UNCONFIRMED stand-in: the names-only WeaponSounds mapping of the attacker weapon)
            if let Some(w) = ev.get("attacker_weapon").and_then(|w| w.as_str()).map(String::from) {
                let ws = st.sounds.entry(w.clone()).or_insert_with(|| mh_audio::WeaponSounds::read(&rd, &w));
                if let Some(cue) = ws.for_event(&ev).map(String::from) {
                    cues.write(mh_audio::PlayCue { cue, pos, ..default() });
                    st.cues_sent += 1;
                }
            }
        }
        st.last_events.push(ev);
        let n = st.last_events.len();
        if n > 20 {
            st.last_events.drain(..n - 20);
        }
    }
}

fn vitals(sim: NonSend<Sim>, pc: Option<Res<crate::input::PlayerControl>>, mut v: ResMut<mh_ui::HudVitals>) {
    let id = pc.map(|p| p.id).unwrap_or(0);
    let views = sim.0.fighters();
    match views.iter().find(|f| f.id == id) {
        Some(f) => {
            v.target = Some(id as u64);
            v.health = f.health.round() as i64;
            v.stamina = f.stamina;
            v.visible = true;
        }
        None => v.visible = false,
    }
}

fn listener(cam: Query<&GlobalTransform, With<crate::camera::FlyCam>>, mut l: ResMut<mh_audio::AudioListener>, time: Res<Time>) {
    if let Some(g) = cam.iter().next() {
        // position, velocity and orientation (mh-audio spatialization needs forward / right; rust-assets r8)
        l.follow(g, time.delta_secs());
    }
}

/// a newly loaded map: its placed AmbientSound / AudioComponents and particle systems start (rust-assets r8)
fn map_ambience(mut commands: Commands, lvl: Res<crate::level::LevelState>, src: Res<crate::source::Source>, mut st: ResMut<BridgeState>, sim: NonSend<Sim>) {
    if !lvl.loaded || lvl.map.is_empty() || st.map_ambience == lvl.map {
        return;
    }
    // audio occlusion (CheckOcclusion 0x2e23880, rust-assets r10) traces the map's collision: the sim's shared world
    // once it is attached to this map
    let Some(cw) = sim.0.collision() else { return };
    commands.insert_resource(mh_audio::occlusion::OcclusionTracer(mh_audio::occlusion::collision_tracer(cw)));
    st.map_ambience = lvl.map.clone();
    let Some(vfs) = &src.vfs else { return };
    let pk = mh_level::Pkgs::new(mh_pak::Reader::new(vfs.clone()));
    let snd = mh_audio::ambient::map_sounds(&pk, &lvl.map);
    let fx = mh_fx::map_fx::map_emitters(&pk, &lvl.map);
    st.map_sounds = snd.len();
    st.map_effects = fx.len();
    commands.insert_resource(mh_audio::MapSounds(snd));
    commands.insert_resource(mh_fx::MapEffects(fx));
    // the map's random ambient spawners and AudioVolumes (ambient zones, LPF, reverb; rust-assets r9)
    let sp = mh_audio::ambient::random_spawners(&pk, &lvl.map);
    // the combat test level has no map package (level.rs TEST_LEVEL)
    let ld = if lvl.map == crate::level::TEST_LEVEL { mh_level::LevelData::default() } else { mh_level::read(&pk, &lvl.map) };
    let vols = mh_audio::ambient::audio_volumes(&pk, &ld);
    st.map_spawners = sp.len();
    st.map_volumes = vols.len();
    commands.insert_resource(mh_audio::MapSpawners(sp));
    commands.insert_resource(mh_audio::MapVolumes(vols));
}

/// Weapon trails (rust-assets r11, mh-fx trail.rs; AMordhauWeapon::StartTrail 0x163fd40 / StopTrails 0x1640370 /
/// UpdateParticleTrails 0x1641540): per fighter, which attack started a trail and whether its blood trail ran
#[derive(Resource, Default)]
pub struct TrailState {
    /// fighter id -> (the attack motion's start time, blood trail started)
    pub active: HashMap<u32, (f64, bool)>,
    pub configs: HashMap<String, mh_fx::trail::TrailConfig>,
    pub starts: usize,
    pub blood: usize,
    pub stops: usize,
}

/// the trail events of the frame's sim events and motions:
/// - "release" (the attack's first release tick: the woosh moment) -> Start { blood: false, duration: the release
///   duration (AttackInfo.Release) }
/// - the first flesh hit of an attack ("hit" by the attacker, surface 1 Flesh) -> Start { blood: true,
///   duration: BloodTrailMaxDuration }
/// - the attack motion left -> Stop { delay 0.15, or 0.25 when another motion follows; 0.3 for a parry / interrupt
///   (Blocked / Parry / Flinch / Stun / Disarmed); rust-assets' StopTrails caller values }
/// - every frame: Sockets { TraceStart, TraceEnd } (Bevy m) of each fighter's weapon
pub fn trail_events(ev: &[serde_json::Value], sim: &mut Sim, st: &mut TrailState, rd: &mh_pak::Reader, out: &mut MessageWriter<mh_fx::trail::TrailEvent>) {
    use mh_fx::trail::TrailEvent;
    let views = sim.0.fighters();
    let mut config = |st: &mut TrailState, id: u32| -> mh_fx::trail::TrailConfig {
        let w = sim.0.weapon_path(id).unwrap_or_default();
        st.configs.entry(w.clone()).or_insert_with(|| mh_fx::trail::TrailConfig::read(rd, &w)).clone()
    };
    for e in ev {
        let s = |k: &str| e.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        match s("kind").as_str() {
            "release" => {
                let Some(id) = sim.0.id_of(&s("who")) else { continue };
                let t = sim.0.combat().zip(sim.0.fighter_index(id)).and_then(|(w, fi)| w.cur_m(fi).map(|m| m.start_time)).unwrap_or(-1.0);
                let duration = e.get("release_duration").and_then(|x| x.as_f64()).unwrap_or(0.0) as f32;
                let config = config(st, id);
                out.write(TrailEvent::Start { weapon: id as u64, blood: false, duration, config });
                st.active.insert(id, (t, false));
                st.starts += 1;
            }
            "hit" if e.get("surface").and_then(|x| x.as_i64()) == Some(1) => {
                let Some(id) = sim.0.id_of(&s("attacker")) else { continue };
                let Some(a) = st.active.get_mut(&id) else { continue };
                if a.1 {
                    continue;
                }
                a.1 = true;
                let config = config(st, id);
                out.write(TrailEvent::Start { weapon: id as u64, blood: true, duration: config.blood_max_duration, config });
                st.blood += 1;
            }
            _ => {}
        }
    }
    // the attack left: its motion is no longer the current one
    let ids: Vec<u32> = st.active.keys().copied().collect();
    for id in ids {
        let (t0, _) = st.active[&id];
        let cur = sim.0.combat().zip(sim.0.fighter_index(id)).and_then(|(w, fi)| w.cur_m(fi).map(|m| (m.start_time, m.attack().is_some(), crate::sim_core::state_name(w, fi))));
        let delay = match &cur {
            Some((t, true, _)) if *t == t0 => continue,
            Some((_, _, k)) if ["Blocked", "Parry", "Flinch", "Stun", "Disarmed"].iter().any(|x| k.starts_with(x)) => 0.3,
            Some(_) => 0.25,
            None => 0.15,
        };
        out.write(TrailEvent::Stop { weapon: id as u64, delay });
        st.active.remove(&id);
        st.stops += 1;
    }
    for v in &views {
        if let Some((a, b)) = sim.0.trace_sockets(v.id) {
            let m = |p: [f32; 3]| Vec3::new(p[0], p[2], p[1]) * 0.01;
            out.write(TrailEvent::Sockets { weapon: v.id as u64, a: m(a), b: m(b) });
        }
    }
}

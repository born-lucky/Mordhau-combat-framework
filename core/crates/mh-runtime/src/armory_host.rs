//! The Armory (Mercenaries) host side, over rust-armory's mh_ui::armory API (docs/RUST_RUNTIME.md s15):
//!
//! - the level actors the platform Blueprint asks for: BP_MordhauCustomizationPlatform's UpdateCamera /
//!   SpawnCharacterDollIfNone / SpawnEquipment find the map's BP_CharacterCustomizationSpot / BP_EquipmentCustomizationSpot
//!   with GetAllActorsOfClass. The MainMenu map (GameDefaultMap) places one of each (MainMenu.umap
//!   BP_CharacterCustomizationSpot_1189 / BP_EquipmentCustomizationSpot_397); their actor and AttachComponent world
//!   transforms go to the UI VM (mh_ui::armory::set_level_actor).
//! - the 3D preview: Resource mh_ui::armory::ArmoryPreview -> the doll (a BP_CustomizationCharacterDoll: the profile's
//!   parts on the character mesh at the doll actor's transform, the class's CharacterMesh0 relative transform) and the
//!   view through the platform's CustomizationObserver camera (AMordhauCameraManager::EnterCustomization; its FOV 26 =
//!   BP_MordhauCustomizationPlatform:UpdateCamera@1701/@4379; the camera itself is placed by menu.rs place_backdrop).
//!   The doll's "Mandible" location (a bone of UMA_Master_Skeleton; USkinnedMeshComponent::GetSocketLocation falls
//!   back to the bone of that name) goes back to the VM (mh_ui::armory::set_doll_socket), which UpdateCamera /
//!   UpdateCharacterDollRotation read.
//! - Message mh_ui::armory::SpawnProfile (PrepareAndSendCustomizationIfChanged 0x15eb160 ->
//!   AMordhauPlayerController::ServerRequestSetDefaultProfile_Implementation 0x15f8570 -> OnReceivedValidProfile
//!   0x15e6580): the profile becomes the player's PendingCharacterProfile. OnReceivedValidProfile re-equips a live
//!   pawn only when AMordhauGameState::CanImmediatelyChangeProfile (0x1586de0: MatchState == WaitingToStart);
//!   otherwise CharacterProfile takes it at the next spawn. The runtime has no WaitingToStart phase (no game mode
//!   runs offline yet), so the profile applies at the player's next spawn (fighter.rs spawn_from_queue reads
//!   Selected.player).
//! - the player's next spawn after a death (user report 2026-10-07 "swapping mercenaries still doesn't work": offline
//!   Local Play never respawned the player, so a pick never reached a fighter): AMordhauGameMode::OnKilled
//!   (mh-mode game_mode.rs on_killed_base: NextRespawnTime = now + ScoringDef PlayerRespawnTime, waves ceil(now / t) * t)
//!   -> AMordhauPlayerController::CanAskForSpawn 0x15c8a90 (NextRespawnTime < TimeSeconds) -> AskForSpawn 0x15c4f00 ->
//!   AMordhauGameMode::RestartPlayer 0x15a6d50 (queues the controller) -> AMordhauGameMode::Tick 0x15ab000 ->
//!   PrepareControllerForRespawn 0x15a4190 (CharacterProfile = PendingCharacterProfile when they differ, i.e. the
//!   SpawnProfile pick = Selected.player) -> a new pawn with that profile (FinalizeSpawnedCharacter 0x15938f0 equips
//!   CharacterProfile.GearCustomization.Equipment). The dead pawn stays as the corpse; the player controls the new one.
//!   UNCONFIRMED stand-ins: the spawn point (round robin over the map's PlayerStarts, not ChoosePlayerStart 0x15876e0);
//!   the ask is made as soon as CanAskForSpawn allows (the game asks on the spectator's primary action,
//!   AMordhauSpectator::SpectatorAction 0x15feff0 -> OnSpectatorAction(0); mh-mode player_tick makes the same choice).

use bevy::math::{DMat3, DQuat, DVec3};
use bevy::prelude::*;
use mh_character::ue::FVector;
use serde_json::json;

/// the preview doll's profile key (loadout.rs register_profile; one key, rebuilt when the profile changes)
pub const PREVIEW_KEY: usize = crate::loadout::CUSTOM_KEY + 999;
/// BP_CustomizationCharacterDoll (the class BP_MordhauCustomizationPlatform spawns as its CharacterDoll)
pub const DOLL_BP: &str = "Mordhau/Content/Mordhau/Blueprints/Characters/BP_CustomizationCharacterDoll";

#[derive(Resource, Default, Clone, Debug, serde::Serialize)]
pub struct ArmoryHost {
    /// the customization spots handed to the UI VM (class, actor name)
    pub spots: Vec<(String, String)>,
    pub spots_fed: bool,
    /// the preview doll's profile (JSON text) as last built
    #[serde(skip)]
    pub preview_json: String,
    pub preview_builds: u32,
    #[serde(skip)]
    pub doll: Option<Entity>,
    pub doll_shown: bool,
    pub mandible_ue: Option<[f32; 3]>,
    /// SpawnProfile messages taken: (picker index, name, profile key)
    pub spawn_profiles: Vec<(usize, String, usize)>,
    pub note: Option<String>,
    /// the doll the held items were built on (doll_weapons), and the items
    #[serde(skip)]
    pub weapons_on: Option<Entity>,
    pub doll_items: Vec<String>,
    /// the player's death -> respawn flow (module doc): NextRespawnTime (app seconds) while the player's pawn is dead
    pub next_respawn_time: Option<f64>,
    /// player respawns done: (new fighter id, profile key, profile name)
    pub player_respawns: Vec<(u32, usize, String)>,
}

pub struct ArmoryHostPlugin;

impl Plugin for ArmoryHostPlugin {
    fn build(&self, app: &mut App) {
        app.insert_non_send(DollAnimState::default());
        app.init_resource::<ArmoryHost>().add_systems(Update, (feed_spots, spawn_profile, player_respawn, preview_doll, doll_weapons, doll_poses, doll_socket).chain());
    }
}

/// mh-level world transform (UE space, column vectors) -> the UI VM's FTransform
pub fn ui_xf(x: &mh_level::Xf) -> mh_ui::armory::Xf {
    let m = &x.m;
    let col = |j: usize| DVec3::new(m[0][j], m[1][j], m[2][j]);
    let (c0, c1, c2) = (col(0), col(1), col(2));
    let s = DVec3::new(c0.length(), c1.length(), c2.length());
    let nz = |v: DVec3, l: f64| if l > 0.0 { v / l } else { v };
    let q = DQuat::from_mat3(&DMat3::from_cols(nz(c0, s.x), nz(c1, s.y), nz(c2, s.z))).normalize();
    let t = x.translation();
    mh_ui::armory::Xf {
        t: FVector::new(t[0] as f32, t[1] as f32, t[2] as f32),
        q: [q.x as f32, q.y as f32, q.z as f32, q.w as f32],
        s: FVector::new(s.x as f32, s.y as f32, s.z as f32),
    }
}

/// the map's customization spots: (class, actor name, actor world, AttachComponent world)
pub fn customization_spots(pk: &mh_level::Pkgs, map: &str) -> Vec<(String, String, mh_level::Xf, mh_level::Xf)> {
    let ex = pk.load_pkg(map);
    let mut out = Vec::new();
    for e in ex.iter() {
        let ty = e.get("Type").and_then(|t| t.as_str()).unwrap_or("");
        if ty != "BP_CharacterCustomizationSpot_C" && ty != "BP_EquipmentCustomizationSpot_C" {
            continue;
        }
        let p = pk.props(e);
        let (Some(root), Some(att)) = (pk.obj(p.get("RootComponent")), pk.obj(p.get("AttachComponent"))) else { continue };
        let name = e.get("Name").and_then(|n| n.as_str()).unwrap_or("").to_string();
        out.push((ty.to_string(), name, pk.world_xf(&root), pk.world_xf(&att)));
    }
    out
}

/// once the backdrop map is in and the UI VM has its world: the spots to the VM
fn feed_spots(rt: Option<NonSendMut<mh_ui::real::Rt>>, bd: Option<Res<crate::menu::MenuBackdrop>>, lvl: Res<crate::level::LevelState>, src: Res<crate::source::Source>, mut host: ResMut<ArmoryHost>) {
    let (Some(mut rt), Some(bd)) = (rt, bd) else { return };
    if host.spots_fed || bd.map.is_empty() || lvl.map != bd.map || !lvl.loaded || !rt.ui.vm.world.contains_key("map") {
        return;
    }
    let Some(v) = &src.vfs else { return };
    host.spots_fed = true;
    let pk = mh_level::Pkgs::new(mh_pak::Reader::new(v.clone()));
    for (class, name, actor, attach) in customization_spots(&pk, &bd.map) {
        mh_ui::armory::set_level_actor(&mut rt.ui.vm, &class, ui_xf(&actor), ui_xf(&attach));
        host.spots.push((class, name));
    }
    if host.spots.is_empty() {
        host.note = Some(format!("{}: no customization spots", bd.map));
    }
}

/// the profile's gear for the sim (weapon / left hand / wearable classes), as main.rs builds it for --profile
fn gear_for(src: &crate::source::Source, spec: Option<&crate::specdata::SpecData>, key: usize, prof: &serde_json::Value) -> Option<crate::loadout::ProfileGear> {
    let v = src.vfs.as_ref()?;
    let pk = mh_level::Pkgs::new(mh_pak::Reader::new(v.clone()));
    let mut g = crate::loadout::profile_gear_value(&pk, key, prof).ok()?;
    if let Some(sd) = spec {
        if let Ok(lo) = crate::loadout::resolve_value(&pk, sd, prof) {
            g.wearables = lo.classes.clone();
        }
    }
    Some(g)
}

/// SpawnProfile -> the player's pending profile (applies at the player's next spawn; see the module doc)
#[allow(clippy::too_many_arguments)]
fn spawn_profile(
    mut ev: MessageReader<mh_ui::armory::SpawnProfile>,
    mut sel: ResMut<crate::loadout::Selected>,
    src: Res<crate::source::Source>,
    spec: Option<Res<crate::specdata::SpecData>>,
    pb: Option<ResMut<crate::fighter::PakBody>>,
    mut host: ResMut<ArmoryHost>,
) {
    let mut pb = pb;
    for m in ev.read() {
        let key = crate::loadout::CUSTOM_KEY + m.index;
        crate::loadout::register_profile(key, m.profile.clone());
        match gear_for(&src, spec.as_deref(), key, &m.profile) {
            Some(g) => {
                sel.gear.insert(key, g);
            }
            None => host.note = Some(format!("SpawnProfile {} {}: no gear", m.index, m.name)),
        }
        // a part set built for an earlier version of this key is stale (bot_skins rebuilds it on spawn)
        if let Some(pb) = pb.as_deref_mut() {
            pb.skins.remove(&key);
        }
        sel.player = key;
        host.spawn_profiles.push((m.index, m.name.clone(), key));
        info!("armory: SpawnProfile {} {} -> profile key {key} (the player's next spawn)", m.index, m.name);
    }
}

/// ScoringDef PlayerRespawnTime / bPlayersSpawnInWaves of the match's mode (spec ENT_MODE_<prefix>: FFA / SKM / DU /
/// TF 5 s, TDM 10 s in waves, FL 0 s in waves); the mode is the Local Play pick (MenuState.started) or the map name's
/// prefix ("FFA_Arena" -> FFA)
fn respawn_rule(spec: Option<&crate::specdata::SpecData>, menu: Option<&crate::menu::MenuState>, map: &str) -> Option<(f64, bool)> {
    let base = map.rsplit('/').next().unwrap_or(map);
    let id = menu.and_then(|m| m.started.clone()).unwrap_or_else(|| base.split('_').next().unwrap_or("").to_string());
    let d = mh_mode::data::ModeData::from_spec(&spec?.m, &id).ok()?;
    Some((d.scoring.player_respawn_time, d.scoring.b_players_spawn_in_waves))
}

/// AMordhauGameMode::RestartPlayer 0x15a6d50 -> PrepareControllerForRespawn 0x15a4190 for the local player: the
/// controller's CharacterProfile takes the pending one (Selected.player, the last SpawnProfile), a new pawn spawns
/// with that profile's gear at (loc, yaw), and the controller possesses it (control rotation = the start's,
/// AGameModeBase::FinishRestartPlayer). The old pawn is left as it is (a corpse). Returns the new fighter id.
pub fn restart_player(sim: &mut crate::sim::Sim, sl: &mut crate::sim::SimLevel, sel: &crate::loadout::Selected, pc: &mut crate::input::PlayerControl, loc: [f32; 3], yaw: f32, team: Option<i64>) -> u32 {
    if let Some(g) = sel.gear.get(&sel.player) {
        sim.0.set_next_weapon(&g.weapon);
        sim.0.set_next_wearables(&g.wearables);
        sim.0.set_next_left(&g.left);
    }
    let id = sim.0.spawn(loc, yaw, team);
    sl.profiles.insert(id, sel.player);
    sl.retired.insert(pc.id);
    pc.id = id;
    pc.yaw = yaw;
    pc.pitch = 0.0;
    pc.yaw_initialised = true;
    id
}

/// the player's pawn died -> NextRespawnTime -> CanAskForSpawn -> AskForSpawn -> RestartPlayer with the pending
/// profile (module doc). The combat test level keeps its own round restart (level.rs test_level_rounds).
#[allow(clippy::too_many_arguments)]
fn player_respawn(
    time: Res<Time>,
    lvl: Res<crate::level::LevelState>,
    menu: Option<Res<crate::menu::MenuState>>,
    spec: Option<Res<crate::specdata::SpecData>>,
    mut sim: NonSendMut<crate::sim::Sim>,
    mut sl: ResMut<crate::sim::SimLevel>,
    sel: Res<crate::loadout::Selected>,
    pc: Option<ResMut<crate::input::PlayerControl>>,
    mut host: ResMut<ArmoryHost>,
) {
    let Some(mut pc) = pc else { return };
    if !lvl.loaded || lvl.map == crate::level::TEST_LEVEL || lvl.starts.is_empty() {
        host.next_respawn_time = None;
        return;
    }
    let views = sim.0.fighters();
    // no pawn yet (the match's first spawn is spawn_from_queue's) or alive: nothing to do
    let Some(me) = views.iter().find(|v| v.id == pc.id) else { return };
    if me.health > 0.0 {
        host.next_respawn_time = None;
        return;
    }
    let now = time.elapsed_secs_f64();
    let nrt = *host.next_respawn_time.get_or_insert_with(|| match respawn_rule(spec.as_deref(), menu.as_deref(), &lvl.map) {
        Some((t, false)) if t > 0.0 => now + t,
        Some((t, true)) if t > 0.0 => (now / t).ceil() * t,
        Some(_) => now,
        // no mode record: the AMordhauGameMode default of every Local Play mode but TDM / FL (UNCONFIRMED fallback)
        None => now + 5.0,
    });
    // CanAskForSpawn: NextRespawnTime < TimeSeconds
    if nrt >= now {
        return;
    }
    let s = &lvl.starts[views.len() % lvl.starts.len()];
    let (loc, yaw) = crate::fighter::to_ue(&s.xf);
    let id = restart_player(&mut sim, &mut sl, &sel, &mut pc, loc, yaw, s.team);
    let name = sel.names.get(sel.player).cloned().or_else(|| host.spawn_profiles.iter().rev().find(|p| p.2 == sel.player).map(|p| p.1.clone())).unwrap_or_default();
    host.player_respawns.push((id, sel.player, name.clone()));
    host.next_respawn_time = None;
    info!("player respawn: fighter {id} with profile key {} {name}", sel.player);
}

/// the doll actor's character mesh (BP_CustomizationCharacterDoll CDO "Mesh" = CharacterMesh0 over its BP template
/// chain) relative to the actor
fn doll_mesh_rel(pk: &mh_level::Pkgs) -> Option<mh_level::Xf> {
    let d = pk.defaults(DOLL_BP);
    let m = pk.obj(d.get("Mesh"))?;
    Some(mh_level::xf::rel_xf(&pk.props(&m)))
}

/// ArmoryPreview -> the doll on the front end (built when the profile changes, moved every frame, despawned when the
/// preview closes)
#[allow(clippy::too_many_arguments)]
fn preview_doll(
    mut commands: Commands,
    prev: Option<Res<mh_ui::armory::ArmoryPreview>>,
    screen: Option<Res<mh_ui::Screen>>,
    mut host: ResMut<ArmoryHost>,
    pb: Option<ResMut<crate::fighter::PakBody>>,
    src: Res<crate::source::Source>,
    spec: Option<Res<crate::specdata::SpecData>>,
    defaults: Option<Res<crate::uetint::Defaults>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut tint: ResMut<Assets<crate::uetint::UeTintMaterial>>,
    mut stdm: ResMut<Assets<StandardMaterial>>,
    mut ibps: ResMut<Assets<bevy::mesh::skinning::SkinnedMeshInverseBindposes>>,
    mut roots: Query<&mut Transform>,
) {
    let on_menu = screen.as_deref().is_some_and(|s| *s == mh_ui::Screen::MainMenu);
    let want = prev.as_deref().filter(|p| p.active && on_menu).and_then(|p| p.profile.clone().map(|j| (p.clone(), j)));
    let Some((p, prof)) = want else {
        if let Some(e) = host.doll.take() {
            commands.entity(e).despawn();
        }
        host.doll_shown = false;
        return;
    };
    let Some(mut pb) = pb else { return };
    let text = prof.to_string();
    if text != host.preview_json || !pb.skins.contains_key(&PREVIEW_KEY) {
        let (Some(vfs), Some(ps)) = (&src.vfs, &src.pak) else { return };
        crate::loadout::register_profile(PREVIEW_KEY, prof.clone());
        let rd = mh_pak::Reader::new(vfs.clone());
        let res = mh_assets::material::Resolver::new(&rd, &**ps);
        let mut st = crate::paksrc::TexStats::default();
        let (skins, info) = crate::fighter::build_profile_skins(PREVIEW_KEY, &mut crate::fighter::SkinCtx {
            vfs,
            ps,
            res: &res,
            spec: spec.as_deref(),
            defaults: defaults.as_deref(),
            materials: src.materials,
            meshes: &mut meshes,
            images: &mut images,
            tint: &mut tint,
            stdm: &mut stdm,
            ibps: &mut ibps,
            st: &mut st,
        });
        // the previous preview's part set goes (its handles drop with it)
        pb.skins.insert(PREVIEW_KEY, skins);
        if let Some(m) = pb.info.get_mut("loadouts").and_then(|l| l.as_object_mut()) {
            m.insert("armory_preview".into(), info);
        }
        host.preview_json = text;
        host.preview_builds += 1;
        if let Some(e) = host.doll.take() {
            commands.entity(e).despawn();
        }
        host.doll_items.clear();
    }
    // the doll actor's world transform (UE location + rotator), as the platform Blueprint leaves it
    let [x, y, z] = p.doll_location;
    let [pitch, yaw, roll] = p.doll_rotation;
    let xf = crate::ue::xf(Some(&json!({"X": x, "Y": y, "Z": z})), crate::ue::rot_quat(Some(&json!({"Pitch": pitch, "Yaw": yaw, "Roll": roll}))), None);
    let tr = Transform::from_matrix(Mat4::from(xf));
    match host.doll {
        Some(e) => {
            if let Ok(mut t) = roots.get_mut(e) {
                *t = tr;
            }
        }
        None => {
            let mesh_tr = src
                .vfs
                .as_ref()
                .and_then(|v| doll_mesh_rel(&mh_level::Pkgs::new(mh_pak::Reader::new(v.clone()))))
                .map(|m| Transform::from_matrix(Mat4::from(crate::plan::xf_gltf(&m))))
                .unwrap_or_else(|| Transform::from_rotation(crate::fighter::yaw_quat(crate::fighter::MESH_YAW)));
            // the doll stands in the character Anim Blueprint's idle for what it holds (DollPose / doll_poses)
            let g = gear_for(&src, spec.as_deref(), PREVIEW_KEY, &prof);
            let pose = DollPose { right: g.as_ref().map(|g| g.weapon.clone()).unwrap_or_default(), left: g.as_ref().map(|g| g.left.clone()).unwrap_or_default() };
            let root = commands.spawn((tr, Visibility::default(), Name::new("ArmoryDoll"), pose)).id();
            crate::fighter::spawn_pak_body(&mut commands, &pb, root, mesh_tr, false, PREVIEW_KEY, u32::MAX);
            host.doll = Some(root);
        }
    }
    host.doll_shown = true;
}

/// the doll's held items: the profile's weapon in the right hand and the spawn's left-hand item (loadout.rs
/// ProfileGear, the same choice the match spawn makes; UMordhauSingleton::ApplyProfileTo PickUp, ported in
/// rust-armory's armory_natives), each a child of its socket joint with UEquipmentSystemComponent::ComputeGrippedTransform
/// rva=0x14b70f0 as mh-sim ports it (pose::gripped: GripLocationLocal, the grip pitch constant, RotationOffset negated
/// for the left socket, RightHandEquipOffset; or the EquippedOffset alone when bUseEquippedOffset). Rebuilt with the
/// doll (its joints carry them). The weapon-specific idle pose (the doll's anim Blueprint) is not ported: the doll
/// keeps the decoded idle clip (UNCONFIRMED).
#[allow(clippy::too_many_arguments)]
fn doll_weapons(
    mut commands: Commands,
    mut host: ResMut<ArmoryHost>,
    src: Res<crate::source::Source>,
    spec: Option<Res<crate::specdata::SpecData>>,
    defaults: Option<Res<crate::uetint::Defaults>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut images: ResMut<Assets<Image>>,
    mut tint: ResMut<Assets<crate::uetint::UeTintMaterial>>,
    mut stdm: ResMut<Assets<StandardMaterial>>,
    bodies: Query<(&ChildOf, &crate::fighter::BodyJoints)>,
) {
    let Some(doll) = host.doll else { return };
    if host.weapons_on == Some(doll) {
        return;
    }
    let Some((_, bj)) = bodies.iter().find(|(c, _)| c.parent() == doll) else { return };
    let Ok(prof) = serde_json::from_str::<serde_json::Value>(&host.preview_json) else { return };
    let Some(sd) = spec.as_deref() else { return };
    host.weapons_on = Some(doll);
    let Some(g) = gear_for(&src, Some(sd), PREVIEW_KEY, &prof) else { return };
    let Some(vfs) = &src.vfs else { return };
    let rd = mh_pak::Reader::new(vfs.clone());
    let pitch = |n: &str| sd.f(&format!("ENT_CONST_{n}"), "FLD_CONST_VALUE", 0.0) as f32;
    let items = [(g.weapon.clone(), g.skin, g.parts.clone(), g.pattern, g.colors, false), (g.left.clone(), g.left_skin, g.left_parts.clone(), g.left_pattern, g.left_colors, true)];
    for (wp, skin, parts, pat, cols, left) in items {
        if wp.is_empty() {
            continue;
        }
        let id = format!("ENT_WPN_{}", wp.rsplit('/').next().unwrap_or(&wp));
        let v3 = |k: &str| {
            let a = sd.v3(&id, &format!("FLD_WPN_{k}"), [0.0; 3]);
            mordhau_core::ue::FVector::new(a[0] as f32, a[1] as f32, a[2] as f32)
        };
        let right_handed = sd.value(&id, "FLD_WPN_EQUIP_B_IS_RIGHT_HANDED").and_then(|v| v.as_bool()).unwrap_or(true);
        // the left-hand item sits on LeftWeapon; a weapon that is not right-handed too
        let on_left = left || !right_handed;
        let sock = if on_left { "LeftWeapon" } else { "RightWeapon" };
        let Some(j) = bj.names.iter().position(|n| n.eq_ignore_ascii_case(sock)).map(|i| bj.joints[i]) else {
            host.note = Some(format!("doll: no {sock} joint"));
            continue;
        };
        let local = match mh_sim::load::equipped_offset(&rd, &wp) {
            Some(eo) => eo,
            None => {
                let r = v3("EQUIP_ROTATION_OFFSET");
                let rot = if on_left { [-r.x, -r.y, -r.z] } else { [r.x, r.y, r.z] };
                let p = pitch(if on_left { "PLAYER_grip_pitch_left" } else { "PLAYER_grip_pitch_right" });
                mh_sim::pose::gripped(&mordhau_core::ue::FTransform::IDENTITY, v3("EQUIP_RIGHT_HAND_EQUIP_OFFSET"), rot, p, v3("EQUIP_GRIP_LOCATION_LOCAL"))
            }
        };
        // AMordhauEquipment::UpdateMaterial's colours / pattern / emblem (mh_assets::equipment_paint, rust-assets r11)
        let paint = mh_assets::equipment_paint::paint(&rd, &wp, skin, pat, cols, g.emblem, g.emblem_colors);
        let (look, _) = crate::weapon::build_look(&wp, skin, &parts, &src, &mut meshes, &mut images, &mut tint, &mut stdm, defaults.as_deref(), Some(&paint));
        let Some(look) = look else { continue };
        let e = commands.spawn((crate::fighter::ue_to_bevy(&local), Visibility::default(), Name::new(format!("ArmoryDoll {sock}")), ChildOf(j))).id();
        for (m, mat) in &look.prims {
            let mut c = commands.spawn((Mesh3d(m.clone()), Transform::default(), ChildOf(e)));
            match mat {
                crate::level::MatH::Std(h) => c.insert(MeshMaterial3d(h.clone())),
                crate::level::MatH::Tint(h) => c.insert(MeshMaterial3d(h.clone())),
            };
        }
        host.doll_items.push(wp.clone());
    }
}

/// A doll character (the Armory preview, the main menu's BP_RandomProfileDoll) posed by the character Anim Blueprint's
/// idle for its equipment: the dolls have no AnimClass of their own (BP_CustomizationCharacterDoll / BP_CharacterDoll /
/// BP_RandomProfileDoll inherit BP_MordhauCharacter's CharacterMesh0 = AB_MordhauCharacterAnimation), so they stand in
/// the in-game idle of what they hold. `right` / `left`: the held equipment's Blueprint packages ("" none).
#[derive(Component, Clone, Debug, Default)]
pub struct DollPose {
    pub right: String,
    pub left: String,
}

/// the doll anim state (NonSend: mh-sim's anim data is Rc-based), built on first use
#[derive(Default)]
pub struct DollAnimState(Option<Result<DollAnim, String>>);

/// the anim data the doll poses share (mh-sim's AB_MordhauCharacterAnimation port)
pub struct DollAnim {
    a: std::rc::Rc<mh_sim::animgraph::AnimAssets>,
    sk: mh_sim::pose::Skeleton,
    spec: mordhau_core::data::Spec,
    upper_w: Vec<f32>,
    /// per doll entity: (equipment key, LowerBody state, upper blend space, upper additive)
    per: std::collections::HashMap<Entity, (String, mh_sim::lower::LowerBody, Option<std::rc::Rc<mh_sim::blendspace::BlendSpace>>, String)>,
}

impl DollAnim {
    fn new(vfs: &std::sync::Arc<mh_pak::Vfs>) -> Result<DollAnim, String> {
        let a = mh_sim::animgraph::AnimAssets::new(vfs.clone())?;
        let rd = mh_pak::Reader::new(vfs.clone());
        // BP_MordhauCharacter CharacterMesh0's SkeletalMesh -> its USkeleton (mh-sim load.rs recipe)
        let (_, mesh, _) = mh_sim::physics::character_mesh(&rd)?;
        let ex = rd.read(&mesh).ok_or_else(|| format!("pak: no mesh {mesh}"))?;
        let skel = ex
            .iter()
            .find(|e| e["Type"].as_str() == Some("SkeletalMesh"))
            .and_then(|e| e["Properties"]["Skeleton"]["ObjectPath"].as_str())
            .map(|p| p.rsplit_once('.').map(|x| x.0).unwrap_or(p).to_string())
            .ok_or("character mesh: no Skeleton")?;
        let src = mh_assets::pak_source::PakSource::new(vfs.clone());
        let (rs, modes) = mh_assets::skeletal_mesh::skeleton(&src, &skel).map_err(|e| e.0)?;
        let target_ref = mh_assets::skeletal_mesh::mesh_reference(&src, &mesh).map_err(|e| e.0)?;
        let sk = mh_sim::pose::Skeleton::from_retargeted_ref(&rs, &modes, &target_ref)?;
        let upper_w = mh_sim::animgraph::mask_weights(&sk, &a.upper_filters);
        Ok(DollAnim { a: std::rc::Rc::new(a), sk, spec: Default::default(), upper_w, per: Default::default() })
    }

    /// the idle pose (local, the skeleton's order) at `now` for the doll's equipment, as FighterAnim::update composes
    /// it without motions: UMordhauAnimInstance::UpdateEquipmentData rva=0x151c4d0 picks the assets (MainEquipment =
    /// the left-hand item when present, else the right; its UpperBlendSpace / LowerAnimation / UpperAdditive, else the
    /// right item's Shield* ones); LowerBody's Ground state at rest; the UpperBody Idle = the upper blend space at
    /// (0, 0) + UpperAdditive (ApplyAdditive_3); LayeredBoneBlend_1 by the upper filters. UNCONFIRMED: the procedural
    /// nodes (RightWeapon base, offhand IK, look-at) are not run on the doll
    fn pose(&mut self, e: Entity, d: &DollPose, now: f64) -> Vec<mordhau_core::ue::FTransform> {
        let a = self.a.clone();
        let key = format!("{}|{}", d.right, d.left);
        let both = !d.left.is_empty() && !d.right.is_empty();
        let main = if d.left.is_empty() { d.right.clone() } else { d.left.clone() };
        let pick = |k: &str, shield: &str| {
            let v = if main.is_empty() { String::new() } else { a.equipment_obj(&main, k) };
            if v.is_empty() && both { a.equipment_obj(&d.right, shield) } else { v }
        };
        let fresh = self.per.get(&e).is_none_or(|x| x.0 != key);
        if fresh {
            let lower = pick("LowerAnimation", "ShieldLowerAnimation");
            let upper = pick("UpperBlendSpace", "ShieldUpperBlendSpace");
            let add = pick("UpperAdditive", "ShieldUpperAdditive");
            let mut lb = mh_sim::lower::LowerBody::new(&a, &lower);
            lb.dedicated_server = false;
            self.per.insert(e, (key, lb, a.blend_space(&upper), add));
        }
        let (_, lb, bs, add) = self.per.get_mut(&e).unwrap();
        let base = lb.pose(&self.spec, &self.sk, &a, now);
        let mut upper = match bs {
            Some(bs) => a.blend_space_pose(&self.sk, bs, 0.0, 0.0, now),
            None => base.clone(),
        };
        if !add.is_empty() {
            if let Some(c) = a.clip(add) {
                let len = c.sequence_length as f64;
                let t = if len > 0.0 { now.rem_euclid(len) } else { 0.0 };
                let ap = mh_sim::additive::additive_sample(&a, &self.sk, add, t);
                mh_sim::additive::apply_additive(&mut upper, &ap, 1.0);
            }
        }
        mh_sim::animgraph::blend_mesh_space(&self.sk, &base, &upper, &self.upper_w)
    }
}

/// every DollPose root: its body's joints from the doll idle (fighter.rs local_pose: component space -> the mesh's
/// Y-up joint locals)
fn doll_poses(
    world_time: Res<Time>,
    src: Res<crate::source::Source>,
    mut anim: NonSendMut<DollAnimState>,
    dolls: Query<(Entity, &DollPose)>,
    bodies: Query<(&ChildOf, &crate::fighter::BodyJoints)>,
    mut tr: Query<&mut Transform>,
    mut host: ResMut<ArmoryHost>,
) {
    if dolls.is_empty() {
        return;
    }
    if anim.0.is_none() {
        let Some(v) = &src.vfs else { return };
        anim.0 = Some(DollAnim::new(v));
        if let Some(Err(e)) = anim.0.as_ref() {
            host.note = Some(format!("doll anim: {e}"));
        }
    }
    let Some(Ok(da)) = anim.0.as_mut() else { return };
    let now = world_time.elapsed_secs_f64();
    for (e, d) in dolls.iter() {
        let Some((_, bj)) = bodies.iter().find(|(c, _)| c.parent() == e) else { continue };
        let local = da.pose(e, d, now);
        let comp = da.sk.to_component(&local);
        let names = da.sk.names.clone();
        let out = crate::fighter::local_pose(bj, &names, &comp);
        for (j, t) in bj.joints.iter().zip(out) {
            if let Ok(mut x) = tr.get_mut(*j) {
                *x = t;
            }
        }
    }
}

/// the doll's "Mandible" (UE cm) to the VM each frame
fn doll_socket(
    rt: Option<NonSendMut<mh_ui::real::Rt>>,
    mut host: ResMut<ArmoryHost>,
    bodies: Query<(&ChildOf, &crate::fighter::BodyJoints)>,
    gt: Query<&GlobalTransform>,
) {
    let (Some(mut rt), Some(doll)) = (rt, host.doll) else { return };
    let Some((_, bj)) = bodies.iter().find(|(c, _)| c.parent() == doll) else { return };
    let Some(j) = bj.names.iter().position(|n| n.eq_ignore_ascii_case("Mandible")).map(|i| bj.joints[i]) else { return };
    let Ok(g) = gt.get(j) else { return };
    let t = g.translation();
    // glTF Y-up metres -> UE cm (fighter.rs to_ue)
    let ue = [t.x * 100.0, t.z * 100.0, t.y * 100.0];
    if ue == [0.0; 3] {
        return;
    }
    mh_ui::armory::set_doll_socket(&mut rt.ui.vm, "Mandible", FVector::new(ue[0], ue[1], ue[2]));
    host.mandible_ue = Some(ue);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// a UE-space rotation + scale survives the matrix -> FTransform conversion
    #[test]
    fn ui_xf_roundtrip() {
        let q = mh_level::xf::rot_quat(Some(&json!({"Pitch": 10.0, "Yaw": -73.0, "Roll": 4.0})));
        let x = mh_level::Xf::trs([10.0, -20.0, 30.0], q, [1.0, 2.0, 1.5]);
        let u = ui_xf(&x);
        assert!((u.t.x - 10.0).abs() < 1e-4 && (u.t.y + 20.0).abs() < 1e-4 && (u.t.z - 30.0).abs() < 1e-4);
        assert!((u.s.y - 2.0).abs() < 1e-4 && (u.s.z - 1.5).abs() < 1e-4);
        let dot = u.q[0] as f64 * q[0] + u.q[1] as f64 * q[1] + u.q[2] as f64 * q[2] + u.q[3] as f64 * q[3];
        assert!(dot.abs() > 0.9999, "quat {:?} vs {:?}", u.q, q);
    }

    /// the MainMenu map places one character and one equipment customization spot (skipped without the install)
    #[test]
    fn main_menu_has_customization_spots() {
        let Ok(vfs) = mh_pak::vfs::Vfs::mount_default() else {
            eprintln!("skip: no Mordhau install");
            return;
        };
        let pk = mh_level::Pkgs::new(mh_pak::Reader::new(std::sync::Arc::new(vfs)));
        let s = customization_spots(&pk, "Mordhau/Content/Mordhau/Maps/MainMenu/MainMenu");
        let classes: Vec<&str> = s.iter().map(|x| x.0.as_str()).collect();
        assert!(classes.contains(&"BP_CharacterCustomizationSpot_C") && classes.contains(&"BP_EquipmentCustomizationSpot_C"), "{classes:?}");
        assert!(doll_mesh_rel(&pk).is_some(), "BP_CustomizationCharacterDoll Mesh");
    }
}

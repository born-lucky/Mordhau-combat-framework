//! The UI runtime: the game's own HUD actor (BP_MordhauHUD, which creates every HUD widget and the main menu in its
//! ReceiveBeginPlay: state/ui_kismet/BP_MordhauHUD.txt ExecuteUbergraph@74..@908) run in the Blueprint VM, proxies for
//! the engine objects the widgets read (player controller, pawn, player / game state, game user settings, the map),
//! per-frame update (latent actions, HUD ReceiveTick, widget Tick, property bindings, Slate layout + paint) and input
//! routed the way Slate routes it (hit test -> hover enter / leave, SButton press / click, key events tunnelling
//! through OnPreviewKeyDown then bubbling OnKeyDown).
//!
//! What the player controller Blueprint does for the UI is replayed here (BP_MordhauPlayerController is gameplay, not
//! run): on its first tick with a HUD, `GetMapName() == "Main Menu"` -> HUD.UseMinimalHUD() + HUD."Show Main Menu"()
//! (BP_MordhauPlayerController:ExecuteUbergraph@5386..@5512); the "Show Main Menu" input action (Escape) ->
//! HUD."Show Main Menu"() (@12145).

use crate::kismet::ObjRef;
use crate::model::*;
use crate::slate::{self, Frame, Hit, Item, Res};
use crate::vm::{Action, Vm};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

pub const HUD_CLASS: &str = "Mordhau/Content/Mordhau/Blueprints/BP_MordhauHUD";
pub const SINGLETON_CLASS: &str = "Mordhau/Content/Mordhau/Blueprints/BP_MordhauSingleton";
pub const GAME_INSTANCE_CLASS: &str = "Mordhau/Content/Mordhau/Blueprints/BP_MordhauGameInstance";
pub const MAIN_MENU_MAP_NAME: &str = "Main Menu";

/// The last frame's output
#[derive(Default, Clone)]
pub struct FrameOut {
    pub items: Vec<Item>,
    pub hits: Vec<Hit>,
    /// (rust-render) UBackgroundBlur elements
    pub blurs: Vec<slate::BlurEl>,
    pub rects: HashMap<Id, slate::Rect>,
    pub popup_rows: Vec<(slate::Rect, Id, usize)>,
    pub parents: HashMap<Id, Id>,
    pub scale: f64,
    pub size: [f64; 2],
}

pub struct UiRuntime {
    pub vm: Vm,
    pub res: Res,
    pub hud: Id,
    pub scale_keys: Vec<(f64, f64)>,
    pub out: FrameOut,
    pub mouse: [f64; 2],
    pub hovered: Option<Id>,
    under: HashSet<Id>,
    pub pressed: Option<Id>,
    began: bool,
    /// the match's game mode Blueprint (None on the menu map)
    pub mode: Option<String>,
    pub players: crate::game::Players,
    /// keys held for the HUD input actions (Show Scoreboard is held)
    held: HashSet<String>,
    /// (perf) seconds spent in update's phases since the last read: VM / actor / HUD ticks, widget Ticks, property
    /// bindings, Slate paint
    pub perf: [f64; 4],
}

impl UiRuntime {
    /// Mount the UI for a map (`map_name`: the map's display name GetMapName returns; "Main Menu" for the menu map)
    pub fn new(vfs: Arc<mh_pak::Vfs>, map_name: &str) -> UiRuntime {
        Self::new_sized(vfs, map_name, None)
    }

    /// `new` with the viewport size known up front: the engine's window exists before any widget is constructed, so
    /// Construct-time reads of it (BP_VideoSettings:UpdateResolutionDropdown -> GetScreenResolution on a fresh
    /// config) see the window, as in the game
    pub fn new_sized(vfs: Arc<mh_pak::Vfs>, map_name: &str, viewport: Option<[f64; 2]>) -> UiRuntime {
        let rd = mh_pak::Reader::new(vfs.clone());
        let mut vm = Vm::new(rd);
        vm.src = Some(mh_assets::pak_source::PakSource::new(vfs.clone()));
        let res = Res::new(vfs.clone());
        let scale_keys = mh_assets::umg::ui_scale_keys(&res.rd);
        for (k, cls) in [
            ("pc", "MordhauPlayerController"),
            ("pawn", "MordhauCharacter"),
            ("ps", "MordhauPlayerState"),
            ("gs", "MordhauGameState"),
            ("settings", "MordhauGameUserSettings"),
            ("input", "MordhauInput"),
            ("map", "World"),
            ("viewport", "GameViewportClient"),
            ("inventory", "MordhauInventory"),
            ("stats", "MordhauStats"),
            // the local player's camera manager (APlayerController::PlayerCameraManager, AMordhauCameraManager):
            // BP_MordhauHUD:HideMainMenu@165 restores the game input mode only when the cast to it succeeds
            ("camera", "MordhauCameraManager"),
        ] {
            let c = vm.native_class(cls);
            let id = vm.new_obj(c, k);
            vm.world.insert(k, id);
        }
        if let Some(sz) = viewport {
            let vpo = vm.world["viewport"];
            vm.set(vpo, "Size", V::st(&[("X", V::Float(sz[0])), ("Y", V::Float(sz[1]))]));
        }
        // the game instance: BP_MordhauGameInstance (Project Settings GameInstanceClass, DefaultEngine.ini; the
        // widgets cast to it), its metadata registered (menu_data.rs RegisterMetadata)
        let gic = vm.bp_class(GAME_INSTANCE_CLASS).unwrap_or_else(|| vm.native_class("MordhauGameInstance"));
        let gi = vm.new_obj(gic, "BP_MordhauGameInstance_C_0");
        vm.world.insert("gi", gi);
        crate::menu_data::register(&mut vm, &vfs, gi);
        // UEngine::GameSingleton: GameSingletonClassName=/Game/Mordhau/Blueprints/BP_MordhauSingleton (DefaultEngine.ini;
        // its CDO holds the button prompt image maps, loadouts, colour tables the widgets read)
        let sc = vm.bp_class(SINGLETON_CLASS).unwrap_or_else(|| vm.native_class("MordhauSingleton"));
        let so = vm.new_obj(sc, "BP_MordhauSingleton_C_0");
        vm.world.insert("singleton", so);
        // UMordhauSingleton config (saved mercenaries, rust-armory): the own config dir's Game.ini, else the CDO's
        crate::armory::load_config(&mut vm, so);
        // the observed character's right-hand equipment (AMordhauCharacter RightHandEquipment, an AMordhauWeapon;
        // BP_Crosshair:UpdateCrosshair@584..@750 reads it and bCanAttack): a melee weapon that can attack until the
        // host says otherwise (set_weapon)
        let wc = vm.native_class("MordhauWeapon");
        let wpn = vm.new_obj(wc, "RightHandEquipment");
        vm.world.insert("weapon", wpn);
        vm.set(wpn, "bCanAttack", V::Bool(true));
        // AMordhauEquipment::AMordhauEquipment 0x1526480: Ammo = MaxAmmo = 0xff (no ammo: BP_EquipmentInfoDisplay hides
        // itself for GetAmmo() == 255, GetVisibility_0@377..@418)
        vm.set(wpn, "Ammo", V::Int(255));
        vm.set(wpn, "MaxAmmo", V::Int(255));
        let pawn = vm.world["pawn"];
        vm.set(pawn, "RightHandEquipment", V::Obj(wpn));
        // a match starts unpossessed (the spawn screen): the HUD is up before the character exists, so its widgets see
        // the no-pawn state first (BP_Crosshair:UpdateCrosshair@565 -> NewMode 1 collapses the ranged reticle)
        let pc = vm.world["pc"];
        vm.set(pc, "Pawn", V::None);
        let m = vm.world["map"];
        vm.set(m, "Name", V::Str(map_name.to_string()));
        // `UMordhauGameInstance::UMordhauGameInstance` 0x1528830: bIsUserControllerPaired = !<platform stub>() where the
        // stub (ICF-folded at 0x7bf520, `xor eax,eax; ret`) is false on PC -> paired from construction, so
        // BP_MainMenu:UpdateInitialInteractionOverlay@0..@264 collapses BP_TitleScreen on the first frame and a PC
        // launch lands on the Home screen. The "press any key" title (OnNewUserSignIn 0x1556030 /
        // OnUserControllerParingConfirmed 0x155d0f0 pairing) is the console flow only.
        vm.set(gi, "bIsUserControllerPaired", V::Bool(true));
        // UGameUserSettings::LoadSettings: the user's GameUserSettings.ini (settings.rs)
        let so = vm.world["settings"];
        crate::settings::load(&mut vm, so);
        let io = vm.world["input"];
        crate::input::load(&mut vm, io);
        // offline backend: everything the menu waits for is available (UNCONFIRMED stand-in for PlayFab)
        for (k, f) in [("inventory", "AreUnlockRecipesAvailable"), ("inventory", "IsInventoryAvailable"), ("stats", "AreStatsAvailable")] {
            let o = vm.world[k];
            vm.set(o, f, V::Bool(true));
        }
        // the match's game mode (GameModeMapPrefixes) and its HUD / GameState / PlayerState classes
        let ini = res.rd.file("Mordhau/Config/DefaultEngine.ini").map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let mode = if map_name == MAIN_MENU_MAP_NAME { None } else { crate::game::mode_for_map(&ini, map_name) };
        let mode_cdo = mode.as_deref().and_then(|m| vm.bp_class(m)).map(|c| c.cdo.clone()).unwrap_or_default();
        let cls_of = |k: &str| mode_cdo.get(k).and_then(crate::game::class_pkg);
        if let Some(gsc) = cls_of("GameStateClass").and_then(|p| vm.bp_class(&p)) {
            let gs = vm.new_obj(gsc, "GameState");
            vm.world.insert("gs", gs);
        }
        let players = crate::game::Players { by_id: HashMap::new(), class: cls_of("PlayerStateClass").or_else(|| Some("Mordhau/Content/Mordhau/Blueprints/BP_MordhauPlayerState".into())) };
        let hud_pkg = cls_of("HUDClass").unwrap_or_else(|| HUD_CLASS.to_string());
        let hud_cls = vm.bp_class(&hud_pkg).or_else(|| vm.bp_class(HUD_CLASS)).unwrap_or_else(|| vm.native_class("MordhauHUD"));
        let hud = vm.new_obj(hud_cls, "HUD_0");
        vm.world.insert("hud", hud);
        let pc = vm.world["pc"];
        vm.set(pc, "MyHUD", V::Obj(hud));
        vm.set(hud, "PlayerOwner", V::Obj(pc));
        UiRuntime { vm, res, hud, scale_keys, out: FrameOut::default(), mouse: [-1.0, -1.0], hovered: None, under: HashSet::new(), pressed: None, began: false, mode, players, held: HashSet::new(), perf: [0.0; 4] }
    }

    /// AHUD BeginPlay -> BP ReceiveBeginPlay (creates the HUD widgets and the hidden main menu); then the player
    /// controller's first-tick main-menu check
    pub fn begin(&mut self) {
        if self.began {
            return;
        }
        self.began = true;
        self.vm.event(self.hud, "ReceiveBeginPlay", vec![]);
        let m = self.vm.world["map"];
        if self.vm.prop(m, "Name").s() == MAIN_MENU_MAP_NAME {
            self.vm.event(self.hud, "UseMinimalHUD", vec![]);
            self.show_main_menu();
        }
    }

    /// HUD "Show Main Menu" (the Escape / pause menu in a match, the front end on the menu map)
    pub fn show_main_menu(&mut self) {
        self.vm.event(self.hud, "Show Main Menu", vec![]);
    }
    pub fn hide_main_menu(&mut self) {
        self.vm.event(self.hud, "HideMainMenu", vec![]);
    }
    pub fn main_menu(&self) -> Option<Id> {
        self.vm.prop(self.hud, "MainMenu").obj()
    }
    pub fn main_menu_visible(&self) -> bool {
        self.main_menu().is_some_and(|m| {
            let v = self.vm.prop(m, "Visibility").i();
            v != 1 && v != 2
        })
    }

    /// the observed character's vitals (AMordhauCharacter Health / Stamina, bytes) as the status bar reads them
    pub fn set_vitals(&mut self, health: i64, stamina: i64, alive: bool) {
        let p = self.vm.world["pawn"];
        // a living character is possessed; a dead one is gone (AController::UnPossess on death, the player spectates)
        let pc = self.vm.world["pc"];
        self.vm.set(pc, "Pawn", if alive { V::Obj(p) } else { V::None });
        self.vm.set(p, "Health", V::Int(health));
        self.vm.set(p, "Stamina", V::Int(stamina));
        self.vm.set(p, "bIsDead", V::Bool(!alive));
    }

    // ---- match data (the HUD Blueprints' own entry points) ---------------------------------------------------------

    /// the players (PlayerStates + GameState.PlayerArray; the scoreboard reads them on Show)
    pub fn set_players(&mut self, ps: &[crate::game::PlayerInfo]) {
        self.players.set(&mut self.vm, ps);
    }

    /// BP_MordhauHUD:SendMessageToKillFeed(Killer PlayerState, KilledBy text, Victim PlayerState) ->
    /// BP_KillFeed:OnMessageReceived
    pub fn kill_feed(&mut self, killer: Option<u64>, with: &str, victim: u64) {
        let k = killer.map(|id| self.players.get(id)).unwrap_or_default();
        let v = self.players.get(victim);
        self.vm.event(self.hud, "SendMessageToKillFeed", vec![k, V::Text(with.to_string()), v]);
    }

    /// a player's chat line: APlayerController::ClientMessage(S, Type, ...) in BP_MordhauPlayerController's ubergraph
    /// -> HUD.ChatBox.OnMessageReceived(Conv_StringToText(S), SenderPlayerState, "ALL" | "TEAM") (@16498 / @16318);
    /// the chat box formats "{prefix} {name}      {message}" under the name plate (BP_ChatBoxEntry:SetupEntry)
    pub fn player_chat(&mut self, sender_id: u64, msg: &str, team: bool) {
        let ps = self.players.get(sender_id);
        let Some(cb) = self.vm.prop(self.hud, "ChatBox").obj() else { return };
        self.vm.event(cb, "OnMessageReceived", vec![V::Text(msg.to_string()), ps, V::Text(if team { "TEAM" } else { "ALL" }.into())]);
    }

    /// BP_MordhauHUD:SendMessageToChatbox(CharacterName, Message)
    pub fn chat(&mut self, name: &str, msg: &str) {
        self.vm.event(self.hud, "SendMessageToChatbox", vec![V::Text(name.to_string()), V::Text(msg.to_string())]);
    }

    /// BP_MordhauHUD:ShowAnnouncement(Text, Subtext, Duration, Type E_AnnouncementType)
    pub fn announce(&mut self, text: &str, sub: &str, duration: f64, kind: i64) {
        self.vm.event(self.hud, "ShowAnnouncement", vec![V::Text(text.into()), V::Text(sub.into()), V::Float(duration), V::Int(kind)]);
    }

    /// BP_MordhauHUD:ShowMatchResult(IsVictory, MainText, SubText) (BP_VictoryPopup / BP_DefeatPopup)
    pub fn match_result(&mut self, victory: bool, main: &str, sub: &str) {
        self.vm.event(self.hud, "ShowMatchResult", vec![V::Bool(victory), V::Text(main.into()), V::Text(sub.into())]);
    }

    /// the local player dealt damage: BP_MordhauPlayerController ClientReceiveScoreBP, Reason Damage (4):
    /// @21652 HUD.ScoreFeed.AddDamage(ScoreAmount, ReasonParam) then @20649 HUD.Crosshair.ShowHitMarker(ReasonParam)
    /// (ReasonParam: head 1, leg 2, else 0; the SC_HitIndicator sound is played by the runtime bridge)
    pub fn dealt_damage(&mut self, amount: i64, param: i64) {
        if let Some(sf) = self.vm.prop(self.hud, "ScoreFeed").obj() {
            self.vm.event(sf, "AddDamage", vec![V::Int(amount), V::Int(param)]);
        }
        self.hit_marker(param);
    }

    /// BP_Crosshair:ShowHitMarker(HitZoneParam) on the HUD's crosshair
    pub fn hit_marker(&mut self, zone: i64) {
        if let Some(c) = self.vm.prop(self.hud, "Crosshair").obj() {
            self.vm.event(c, "ShowHitMarker", vec![V::Int(zone)]);
        }
    }

    /// BP_Crosshair:TriggerDamageIndicator(Render Angle): the direction damage came from, degrees
    pub fn damage_taken(&mut self, angle_deg: f64) {
        if let Some(c) = self.vm.prop(self.hud, "Crosshair").obj() {
            self.vm.event(c, "TriggerDamageIndicator", vec![V::Float(angle_deg)]);
        }
    }

    /// the GameState's match clock (seconds left; AMordhauGameState replicated time the scoreboard reads)
    pub fn set_match_time(&mut self, seconds_left: f64) {
        let gs = self.vm.world["gs"];
        self.vm.set(gs, "RemainingTime", V::Int(seconds_left.max(0.0) as i64));
        self.vm.set(gs, "MatchTimeRemaining", V::Float(seconds_left));
        let el = self.vm.prop(gs, "ElapsedTime").i();
        self.set_match_clock("InProgress", el, el + seconds_left.max(0.0).ceil() as i64);
    }

    /// the GameState fields BP_MordhauGameState:GetScoreboardTime reads: MatchState ("WaitingToStart" ->
    /// WarmupEnd - server time, "InProgress" -> GetScoreboardTimeInProgress = max(MatchDurationMax - ElapsedTime, 0),
    /// or 200 days ("no limit": the scoreboard then shows no clock) when MatchDurationMax <= 0; "WaitingPostMatch" ->
    /// EndMatchMapChangeEnd - server time). Seconds; duration_max 0 = untimed
    pub fn set_match_clock(&mut self, match_state: &str, elapsed: i64, duration_max: i64) {
        let gs = self.vm.world["gs"];
        self.vm.set(gs, "MatchState", V::Name(match_state.to_string()));
        self.vm.set(gs, "ElapsedTime", V::Int(elapsed));
        self.vm.set(gs, "MatchDurationMax", V::Int(duration_max));
    }

    /// the input actions bound to a key (Input.ini [/Script/Mordhau.MordhauInput] ActionMappings)
    fn actions_for(&self, key: &str) -> Vec<String> {
        let io = self.vm.world["input"];
        self.vm.prop(io, "ActionMappings").arr().iter().filter(|m| m.field("Key").field("KeyName").s() == key).map(|m| m.field("ActionName").s()).collect()
    }

    /// what BP_MordhauPlayerController does for the HUD actions: "Show Scoreboard" pressed -> HUD.ShowScoreboard
    /// (ExecuteUbergraph_BP_MordhauPlayerController@10264..@10323), released -> HUD.HideScoreboard (@10052..@10227);
    /// "Show Chat" -> HUD.ShowChatbox(false) (@12107), "Show Team Chat" -> HUD.ShowChatbox(true) (@12069)
    fn hud_action(&mut self, key: &str, pressed: bool) -> bool {
        if self.mode.is_none() || self.main_menu_visible() {
            return false;
        }
        let mut used = false;
        for a in self.actions_for(key) {
            match (a.as_str(), pressed) {
                ("Show Scoreboard", true) => {
                    self.held.insert(key.to_string());
                    self.vm.event(self.hud, "ShowScoreboard", vec![]);
                    used = true;
                }
                ("Show Scoreboard", false) if self.held.remove(key) => {
                    if let Some(sb) = self.vm.prop(self.hud, "Scoreboard").obj() {
                        self.vm.set(sb, "isShowing", V::Bool(false));
                    }
                    self.vm.event(self.hud, "HideScoreboard", vec![]);
                    used = true;
                }
                ("Show Chat", true) => {
                    self.vm.event(self.hud, "ShowChatbox", vec![V::Bool(false)]);
                    used = true;
                }
                ("Show Team Chat", true) => {
                    self.vm.event(self.hud, "ShowChatbox", vec![V::Bool(true)]);
                    used = true;
                }
                // "Show Profile Select" (DefaultInput.ini: B): BP_MordhauPlayerController InpActEvt_Show Profile
                // Select -> HandleShowProfileSelect (ubergraph @29222 -> @30634 -> @26354): with
                // bSendsDefaultCustomization false -> @12386 HUD.Show Profile Customization(1), the in-match
                // Mercenaries list (BP_MordhauHUD:Show Profile Customization@52 -> MainMenu "Show Profile Customization")
                ("Show Profile Select", true) => {
                    let pc = self.vm.world["pc"];
                    if !self.vm.prop(pc, "bSendsDefaultCustomization").truthy() {
                        self.vm.event(self.hud, "Show Profile Customization", vec![V::Int(1)]);
                    }
                    used = true;
                }
                _ => {}
            }
        }
        used
    }

    /// the observed character's right-hand equipment by its Blueprint package (None = empty hands): Ammo / MaxAmmo
    /// from that class's defaults (AMordhauEquipment ctor 0xff unless the Blueprint overrides them), then `ammo`
    /// (the sim's current count) when given. BP_EquipmentInfoDisplay shows the "Ammo a / b" box only when
    /// GetAmmo() != 255 (bows, crossbows, throwables), its text = "{a} / {b}" from GetAmmo / GetCurrentMaxAmmo.
    pub fn set_equipment(&mut self, class_pkg: Option<&str>, can_attack: bool, ammo: Option<i64>) {
        let w = self.vm.world["weapon"];
        let (mut a, mut m) = (255, 255);
        if let Some(c) = class_pkg.and_then(|p| self.vm.bp_class(p)) {
            if let Some(v) = c.cdo.get("Ammo") {
                a = v.i();
            }
            if let Some(v) = c.cdo.get("MaxAmmo") {
                m = v.i();
            }
        }
        if let Some(x) = ammo {
            a = x;
        }
        self.vm.set(w, "Ammo", V::Int(a));
        self.vm.set(w, "MaxAmmo", V::Int(m));
        self.set_weapon(class_pkg.is_some(), can_attack);
        let (pc, p) = (self.vm.world["pc"], self.vm.world["pawn"]);
        self.vm.set(pc, "Pawn", V::Obj(p));
    }

    /// the observed character's weapon: present (None = empty hands) and whether it can attack
    pub fn set_weapon(&mut self, held: bool, can_attack: bool) {
        let p = self.vm.world["pawn"];
        let w = self.vm.world["weapon"];
        self.vm.set(w, "bCanAttack", V::Bool(can_attack));
        self.vm.set(p, "RightHandEquipment", if held { V::Obj(w) } else { V::None });
    }

    pub fn ui_scale(&self, size: [f64; 2]) -> f64 {
        mh_assets::umg::ui_scale(&self.scale_keys, size)
    }

    /// One frame: VM time (latent actions, animation ends), HUD ReceiveTick, widget Tick, bindings, layout + paint
    pub fn update(&mut self, dt: f64, size: [f64; 2]) {
        self.begin();
        let vpo = self.vm.world["viewport"];
        let scale = self.ui_scale(size);
        self.vm.set(vpo, "Size", V::st(&[("X", V::Float(size[0])), ("Y", V::Float(size[1]))]));
        self.vm.set(vpo, "Scale", V::Float(scale));
        self.vm.set(vpo, "Mouse", V::st(&[("X", V::Float(self.mouse[0])), ("Y", V::Float(self.mouse[1]))]));
        // finished news requests reach their widgets on the game thread (news.rs)
        crate::news::poll(&mut self.vm);
        let t0 = std::time::Instant::now();
        self.vm.tick(dt);
        // AActor ticks of VM-spawned actors (customization platform / observer: rust-armory)
        crate::armory::tick(&mut self.vm, dt);
        self.vm.event(self.hud, "ReceiveTick", vec![V::Float(dt)]);
        // UUserWidget::NativeTick -> BP Tick(MyGeometry, InDeltaTime) for widgets painted last frame
        let t1 = std::time::Instant::now();
        let painted: Vec<Id> = {
            let mut v: Vec<Id> = self.out.rects.keys().copied().filter(|&w| self.vm.alive(w) && self.vm.o(w).class.tree_class().is_some()).collect();
            v.sort();
            v
        };
        for w in painted {
            self.vm.event(w, "Tick", vec![V::Struct(Box::default()), V::Float(dt)]);
        }
        let t2 = std::time::Instant::now();
        slate::run_bindings(&mut self.vm, &mut self.res);
        let t3 = std::time::Instant::now();
        let mut f = Frame::new(&mut self.vm, &mut self.res);
        f.hovered = self.hovered;
        f.pressed = self.pressed;
        f.mouse = self.mouse;
        f.paint_viewport(size, scale);
        self.out = FrameOut { items: std::mem::take(&mut f.items), hits: std::mem::take(&mut f.hits), blurs: std::mem::take(&mut f.blurs), rects: std::mem::take(&mut f.rects), popup_rows: std::mem::take(&mut f.popup_rows), parents: std::mem::take(&mut f.parents), scale, size };
        let t4 = std::time::Instant::now();
        for (i, d) in [t1 - t0, t2 - t1, t3 - t2, t4 - t3].into_iter().enumerate() {
            self.perf[i] += d.as_secs_f64();
        }
    }

    pub fn take_actions(&mut self) -> Vec<Action> {
        std::mem::take(&mut self.vm.actions)
    }

    fn contains(r: &slate::Rect, clip: &Option<slate::Rect>, p: [f64; 2]) -> bool {
        let inr = p[0] >= r[0] && p[1] >= r[1] && p[0] < r[0] + r[2] && p[1] < r[1] + r[3];
        let inc = clip.map_or(true, |c| p[0] >= c[0] && p[1] >= c[1] && p[0] < c[0] + c[2] && p[1] < c[1] + c[3]);
        inr && inc
    }

    /// the Slate widget path under the cursor, leaf first: the topmost hit-testable widget (last painted wins,
    /// FHittestGrid) and the widgets it was painted inside, up to its viewport root (events bubble along it)
    pub fn hits_at(&self, p: [f64; 2]) -> Vec<Id> {
        let Some(leaf) = self.out.hits.iter().rev().find(|h| Self::contains(&h.rect, &h.clip, p)).map(|h| h.widget) else { return vec![] };
        let mut path = vec![leaf];
        let mut w = leaf;
        while let Some(&p) = self.out.parents.get(&w) {
            if path.len() > 512 {
                break;
            }
            path.push(p);
            w = p;
        }
        path
    }

    fn pointer(&self, button: &str) -> V {
        V::st(&[
            ("EffectingButton", V::st(&[("KeyName", V::Name(button.into()))])),
            ("ScreenSpacePosition", V::st(&[("X", V::Float(self.mouse[0])), ("Y", V::Float(self.mouse[1]))])),
            // (rust-armory) FPointerEvent PressedButtons (the mouse buttons held, kept by mouse_down / mouse_up)
            ("PressedButtons", self.held_buttons()),
        ])
    }

    /// (rust-armory) the mouse buttons held now (FSlateApplication's PressedMouseButtons), as key names
    fn held_buttons(&self) -> V {
        self.vm.world.get("map").map(|&m| self.vm.prop(m, "__held_buttons")).filter(|v| matches!(v, V::Array(_))).unwrap_or(V::Array(vec![]))
    }

    fn set_held(&mut self, button: &str, down: bool) {
        let Some(&m) = self.vm.world.get("map") else { return };
        let mut v: Vec<V> = self.held_buttons().arr().iter().filter(|b| b.s() != button).cloned().collect();
        if down {
            v.push(V::Name(button.into()));
        }
        self.vm.set(m, "__held_buttons", V::Array(v));
    }

    /// (rust-armory) FSlateApplication::RouteAlongFocusPath / the widget path under the cursor (UE 4.26): a pointer
    /// event bubbles leaf to root through the user widgets' BP handlers (UUserWidget::NativeOnMouseMove /
    /// NativeOnMouseWheel -> OnMouseMove / OnMouseWheel) until one returns Handled; true when handled
    fn bubble_pointer(&mut self, event: &str, ev: &V) -> bool {
        for w in self.hits_at(self.mouse) {
            if let Some(o) = self.vm.event(w, event, vec![V::Struct(Box::default()), ev.clone()]) {
                if o.get("ReturnValue").map(|r| r.field("Handled").truthy()).unwrap_or(false) {
                    return true;
                }
            }
        }
        false
    }

    fn sound(&mut self, w: Id, which: &str) {
        let ws = self.vm.prop(w, "WidgetStyle");
        if let V::Asset(a) = ws.field(which).field("ResourceObject") {
            self.vm.actions.push(Action::Sound(a.package.clone()));
        }
    }

    fn multicast(&mut self, w: Id, del: &str, args: Vec<V>) {
        if let V::Multi(m) = self.vm.prop(w, del) {
            for (o, f) in m {
                self.vm.call_named(o, &f, args.clone());
            }
        }
    }

    fn is_button(&self, w: Id) -> bool {
        matches!(self.vm.o(w).class.native(), "Button" | "CheckBox" | "ComboBoxString" | "ComboBoxText" | "Slider")
    }
    fn enabled(&self, w: Id) -> bool {
        !matches!(self.vm.prop(w, "bIsEnabled"), V::Bool(false))
    }

    /// SSlider::OnMouseButtonDown / OnMouseMove while captured: the value from the cursor's position along the bar
    /// (SSlider::PositionToValue: (x - thumb half) / (width - thumb), clamped 0..1, then the UMG slider's
    /// MinValue..MaxValue and StepSize; UE 4.26 SSlider.cpp, values UNCONFIRMED), OnValueChanged(Value) broadcast
    fn slider_drag(&mut self, w: Id) {
        let Some(r) = self.out.rects.get(&w).copied() else { return };
        let thumb = 8.0 * self.out.scale;
        let k = ((self.mouse[0] - r[0] - thumb * 0.5) / (r[2] - thumb).max(1.0)).clamp(0.0, 1.0);
        let lo = match self.vm.prop(w, "MinValue") {
            V::None => 0.0,
            v => v.f(),
        };
        let hi = match self.vm.prop(w, "MaxValue") {
            V::None => 1.0,
            v => v.f(),
        };
        let mut v = lo + (hi - lo) * k;
        let step = self.vm.prop(w, "StepSize").f();
        if step > 0.0 {
            v = lo + ((v - lo) / step).round() * step;
        }
        if (self.vm.prop(w, "Value").f() - v).abs() > 1e-9 {
            self.vm.set(w, "Value", V::Float(v));
            self.multicast(w, "OnValueChanged", vec![V::Float(v)]);
        }
    }

    /// FSlateApplication::ProcessMouseMoveEvent: hover enter / leave along the widgets under the cursor
    pub fn mouse_move(&mut self, p: [f64; 2]) {
        // (rust-armory) FPointerEvent CursorDelta = this position - the last one
        let delta = [p[0] - self.mouse[0], p[1] - self.mouse[1]];
        self.mouse = p;
        if delta != [0.0, 0.0] {
            let mut ev = self.pointer("None");
            if let V::Struct(m) = &mut ev {
                m.insert("CursorDelta".into(), V::st(&[("X", V::Float(delta[0])), ("Y", V::Float(delta[1]))]));
            }
            self.bubble_pointer("OnMouseMove", &ev);
        }
        if let Some(w) = self.pressed {
            if self.vm.alive(w) && self.vm.o(w).class.native() == "Slider" {
                self.slider_drag(w);
            }
        }
        let under = self.hits_at(p);
        let now: HashSet<Id> = under.iter().copied().collect();
        let left: Vec<Id> = self.under.difference(&now).copied().collect();
        let entered: Vec<Id> = under.iter().rev().copied().filter(|w| !self.under.contains(w)).collect();
        let ev = self.pointer("None");
        for w in left {
            if !self.vm.alive(w) {
                continue;
            }
            self.vm.set(w, "__hovered", V::Bool(false));
            if self.is_button(w) {
                self.multicast(w, "OnUnhovered", vec![]);
            } else {
                self.vm.event(w, "OnMouseLeave", vec![ev.clone()]);
            }
        }
        for w in entered {
            self.vm.set(w, "__hovered", V::Bool(true));
            if self.is_button(w) {
                if self.enabled(w) {
                    // SButton::OnMouseEnter plays HoveredSlateSound
                    self.sound(w, "HoveredSlateSound");
                    self.multicast(w, "OnHovered", vec![]);
                }
            } else {
                self.vm.event(w, "OnMouseEnter", vec![V::Struct(Box::default()), ev.clone()]);
            }
        }
        self.under = now;
        self.hovered = under.into_iter().find(|&w| self.is_button(w));
    }

    /// mouse button down: the topmost button presses (SButton::OnMouseButtonDown, ClickMethod DownAndUp); otherwise
    /// OnMouseButtonDown bubbles through the user widgets under the cursor until one returns Handled
    /// an open combo menu takes the click: a row selects its option (UComboBoxString OnSelectionChanged with
    /// ESelectInfo::OnMouseClick 2), anywhere else closes the menu (Slate dismisses the menu and eats the click)
    fn combo_menu_click(&mut self) -> bool {
        let open: Vec<Id> = self.out.popup_rows.iter().map(|r| r.1).collect();
        if open.is_empty() {
            return false;
        }
        let p = self.mouse;
        let hit = self.out.popup_rows.iter().find(|(r, _, _)| p[0] >= r[0] && p[0] < r[0] + r[2] && p[1] >= r[1] && p[1] < r[1] + r[3]).cloned();
        for c in &open {
            self.vm.set(*c, "__open", V::Bool(false));
        }
        if let Some((_, c, i)) = hit {
            let opt = self.vm.prop(c, "DefaultOptions").arr().get(i).map(V::s).unwrap_or_default();
            let old = self.vm.prop(c, "SelectedOption").s();
            self.vm.set(c, "SelectedOption", V::Str(opt.clone()));
            if old != opt {
                self.multicast(c, "OnSelectionChanged", vec![V::Str(opt), V::Int(2)]);
            }
        }
        self.out.popup_rows.clear();
        true
    }

    /// FSlateApplication::ProcessMouseWheelOrGestureEvent -> SScrollBox::OnMouseWheel: the innermost scroll box under
    /// the cursor scrolls by -delta x Slate.GlobalScrollAmount (32 Slate units, the UE 4.26 cvar default: UNCONFIRMED)
    pub fn mouse_wheel(&mut self, delta: f64) {
        let p = self.mouse;
        let mut best: Option<(Id, f64)> = None;
        for (&w, r) in &self.out.rects {
            // (rust-armory) list views scroll too: STableViewBase::OnMouseWheel (UE 4.26) ScrollBy(-WheelDelta x
            // GetGlobalScrollAmount() x WheelScrollMultiplier) - the armor TileView_Wearables, the loadout ListView
            if !self.vm.alive(w) || !matches!(self.vm.o(w).class.native(), "ScrollBox" | "ListView" | "TileView" | "TreeView") {
                continue;
            }
            if p[0] >= r[0] && p[0] < r[0] + r[2] && p[1] >= r[1] && p[1] < r[1] + r[3] {
                let area = r[2] * r[3];
                if best.is_none_or(|b| area < b.1) {
                    best = Some((w, area));
                }
            }
        }
        // (rust-armory) no scrollable list under the cursor: OnMouseWheel bubbles through the user widgets (the
        // Armory's BP_CustomizationPreview zooms the doll: OnMouseWheel -> OnMouseWheelScrolling)
        let scrollable = best.is_some_and(|(w, _)| self.vm.prop(w, "__scroll_max").f() > 0.0);
        if !scrollable {
            let mut ev = self.pointer("None");
            if let V::Struct(m) = &mut ev {
                m.insert("WheelDelta".into(), V::Float(delta));
            }
            if self.bubble_pointer("OnMouseWheel", &ev) {
                return;
            }
        }
        if let Some((w, _)) = best {
            let mx = self.vm.prop(w, "__scroll_max").f().max(0.0);
            let cur = self.vm.prop(w, "__scroll").f().min(mx);
            // WheelScrollMultiplier: UScrollBox / UListViewBase property, ctor 1 (UE 4.26)
            let mul = match self.vm.prop(w, "WheelScrollMultiplier") {
                V::None => 1.0,
                v => v.f(),
            };
            self.vm.set(w, "__scroll", V::Float((cur - delta * 32.0 * mul).clamp(0.0, mx)));
        }
    }

    /// a UMG pointer-event delegate bound through a property binding (run_bindings): call it, Handled stops routing
    fn bound_pointer(&mut self, w: Id, prop: &str, ev: &V) -> bool {
        if let V::Delegate(o, f) = self.vm.prop(w, prop) {
            if self.vm.alive(o) {
                let r = self.vm.call_named(o, &f, vec![V::Struct(Box::default()), ev.clone()]).0;
                return r.field("Handled").truthy();
            }
        }
        false
    }

    /// FSlateApplication::ProcessMouseButtonDownEvent (UE 4.26): the press is routed (mouse_down_route); when no handler
    /// moved the user focus, the deepest focusable widget under the cursor takes it (EFocusCause::Mouse), which is how
    /// a clicked button puts BP_KeyBindingsSettings on the focus path so its OnPreviewKeyDown sees the next key
    pub fn mouse_down(&mut self, button: &str) {
        self.set_held(button, true);
        let before = self.vm.focus;
        let under = self.hits_at(self.mouse);
        // FSlateApplication::ProcessMouseButtonDownEvent: OnPreviewMouseButtonDown tunnels root -> leaf over the
        // widgets under the cursor before the press routes; BP_KeyBindingsSettings overrides it to take a MOUSE button
        // as the new binding while a row is editing (user report 2026-10-07: rebinding Strike to a mouse button saved
        // Key=None because the preview never reached the widget)
        {
            let ev = self.pointer(button);
            let handled = |o: Option<HashMap<String, V>>| o.and_then(|o| o.get("ReturnValue").map(|r| r.field("Handled").truthy())).unwrap_or(false);
            for &w in under.iter().rev() {
                if self.vm.alive(w) && handled(self.vm.event(w, "OnPreviewMouseButtonDown", vec![V::Struct(Box::default()), ev.clone()])) {
                    return;
                }
            }
        }
        self.mouse_down_route(button);
        if self.vm.focus == before {
            if let Some(&w) = under.iter().find(|&&w| self.focusable(w) && self.vm.visible_widget_path(w).is_some()) {
                self.vm.focus = Some(w);
            }
        }
    }

    /// SWidget::SupportsKeyboardFocus for the UMG widgets the menus use: SButton / SCheckBox / SSlider / SComboButton /
    /// editable text take focus unless IsFocusable is false; a user widget only with bIsFocusable
    fn focusable(&self, w: Id) -> bool {
        if !self.vm.alive(w) {
            return false;
        }
        let o = self.vm.o(w);
        match o.class.native() {
            "Button" | "CheckBox" | "Slider" | "ComboBoxString" | "ComboBoxText" | "EditableText" | "EditableTextBox" | "MultiLineEditableTextBox" => !matches!(self.vm.prop(w, "IsFocusable"), V::Bool(false)),
            _ => o.class.tree_class().is_some() && self.vm.prop(w, "bIsFocusable").truthy(),
        }
    }

    fn mouse_down_route(&mut self, button: &str) {
        if self.combo_menu_click() {
            return;
        }
        let under = self.hits_at(self.mouse);
        let ev = self.pointer(button);
        let mut below: Option<Id> = None;
        for w in under {
            // (rust-armory) an unhandled press reaching a list view through one of its entry widgets is the row's:
            // STableRow::OnMouseButtonDown (UE 4.26) with the left button and SelectionMode Single (UListView default;
            // ESelectionMode None 0 / Single 1 / SingleToggle 2 / Multi 3) selects that row's item when it is not
            // selected yet -> the selection-changed notifications (armory_natives' BP_SetSelectedItem arm)
            if button == "LeftMouseButton" && matches!(self.vm.o(w).class.native(), "ListView" | "TileView" | "TreeView") {
                let mode = match self.vm.prop(w, "SelectionMode") {
                    V::None => 1,
                    v => v.i(),
                };
                let entries = self.vm.prop(w, "__entries").arr().to_vec();
                if let Some(i) = below.and_then(|e| entries.iter().position(|x| x.obj() == Some(e))) {
                    let item = self.vm.prop(w, "ListItems").arr().get(i).cloned().unwrap_or_default();
                    if mode != 0 && !matches!(item, V::None) && !self.vm.prop(w, "__selected").same(&item) {
                        crate::natives::call(&mut self.vm, crate::vm::Ctx::Obj(w), "ListView", "BP_SetSelectedItem", &[item]);
                    }
                    return;
                }
            }
            below = Some(w);
            if self.is_button(w) {
                if !self.enabled(w) {
                    return;
                }
                if button == "LeftMouseButton" {
                    self.pressed = Some(w);
                    self.vm.set(w, "__pressed", V::Bool(true));
                    if self.vm.o(w).class.native() == "Slider" {
                        self.multicast(w, "OnMouseCaptureBegin", vec![]);
                        self.slider_drag(w);
                        return;
                    }
                    self.sound(w, "PressedSlateSound");
                    self.multicast(w, "OnPressed", vec![]);
                }
                return;
            }
            // UBorder / UImage HandleMouseButtonDown: the bound OnMouseButtonDownEvent(Geometry, MouseEvent) -> its FEventReply
            if self.bound_pointer(w, "OnMouseButtonDownEvent", &ev) {
                return;
            }
            if let Some(o) = self.vm.event(w, "OnMouseButtonDown", vec![V::Struct(Box::default()), ev.clone()]) {
                if o.get("ReturnValue").map(|r| r.field("Handled").truthy()).unwrap_or(false) {
                    return;
                }
            }
        }
    }

    /// mouse button up: a pressed button still under the cursor clicks (OnReleased, OnClicked; CheckBox toggles)
    pub fn mouse_up(&mut self, button: &str) {
        self.set_held(button, false);
        let Some(w) = self.pressed.take() else {
            let under = self.hits_at(self.mouse);
            let ev = self.pointer(button);
            for w in under {
                if self.bound_pointer(w, "OnMouseButtonUpEvent", &ev) {
                    return;
                }
                if let Some(o) = self.vm.event(w, "OnMouseButtonUp", vec![V::Struct(Box::default()), ev.clone()]) {
                    if o.get("ReturnValue").map(|r| r.field("Handled").truthy()).unwrap_or(false) {
                        return;
                    }
                }
            }
            return;
        };
        self.vm.set(w, "__pressed", V::Bool(false));
        if self.vm.o(w).class.native() == "Slider" {
            self.multicast(w, "OnMouseCaptureEnd", vec![]);
            return;
        }
        self.multicast(w, "OnReleased", vec![]);
        if self.hits_at(self.mouse).contains(&w) {
            match self.vm.o(w).class.native() {
                // SComboButton: a click toggles its menu (opened above everything next frame)
                "ComboBoxString" | "ComboBoxText" => {
                    let o = self.vm.prop(w, "__open").truthy();
                    self.vm.set(w, "__open", V::Bool(!o));
                    self.multicast(w, "OnOpening", vec![]);
                }
                "CheckBox" => {
                    let c = self.vm.prop(w, "CheckedState").i() == 1;
                    self.vm.set(w, "CheckedState", V::Int(if c { 0 } else { 1 }));
                    self.multicast(w, "OnCheckStateChanged", vec![V::Bool(!c)]);
                }
                _ => self.multicast(w, "OnClicked", vec![]),
            }
        }
    }

    pub fn click(&mut self, p: [f64; 2]) {
        self.mouse_move(p);
        self.mouse_down("LeftMouseButton");
        self.mouse_up("LeftMouseButton");
    }

    /// the user widgets on the focus path (root first): the focused widget's chain, else the visible viewport roots
    fn focus_path(&self) -> Vec<Id> {
        let mut path = vec![];
        if let Some(visible) = self.vm.focus.and_then(|f| self.vm.visible_widget_path(f)) {
            path = visible;
            return path;
        }
        // in a match, no focused widget = the game viewport has the user focus (FInputModeGameOnly, the input mode
        // BP_MainMenu:Hide / the HUD restore): keys go to the player controller's input actions, not to widgets
        if self.mode.is_some() {
            return path;
        }
        let mut roots = self.vm.viewport.clone();
        roots.sort_by_key(|x| -x.1);
        for (w, _) in roots {
            let v = self.vm.prop(w, "Visibility").i();
            if v != 1 && v != 2 {
                path.push(w);
            }
        }
        path
    }

    /// FSlateApplication::ProcessKeyDownEvent: OnPreviewKeyDown tunnels root -> leaf, OnKeyDown bubbles leaf -> root
    pub fn key_down(&mut self, key: &str) {
        if self.hud_action(key, true) {
            return;
        }
        let ev = V::st(&[("Key", V::st(&[("KeyName", V::Name(key.into()))]))]);
        let path = self.focus_path();
        let handled = |o: Option<HashMap<String, V>>| o.and_then(|o| o.get("ReturnValue").map(|r| r.field("Handled").truthy())).unwrap_or(false);
        for &w in &path {
            if handled(self.vm.event(w, "OnPreviewKeyDown", vec![V::Struct(Box::default()), ev.clone()])) {
                return;
            }
        }
        for &w in path.iter().rev() {
            if handled(self.vm.event(w, "OnKeyDown", vec![V::Struct(Box::default()), ev.clone()])) {
                return;
            }
        }
        // a focused button clicks on Enter / Space (SButton::OnKeyDown)
        if let Some(f) = self.vm.focus {
            if (key == "Enter" || key == "SpaceBar") && self.vm.alive(f) && self.vm.o(f).class.native() == "Button" {
                self.multicast(f, "OnClicked", vec![]);
                return;
            }
        }
        // the player controller's "Show Main Menu" action (DefaultInput.ini: Escape) when nothing handled it
        if key == "Escape" && !self.main_menu_visible() {
            self.show_main_menu();
        }
    }

    pub fn key_up(&mut self, key: &str) {
        if self.hud_action(key, false) {
            return;
        }
        let ev = V::st(&[("Key", V::st(&[("KeyName", V::Name(key.into()))]))]);
        let path = self.focus_path();
        for &w in path.iter().rev() {
            if let Some(o) = self.vm.event(w, "OnKeyUp", vec![V::Struct(Box::default()), ev.clone()]) {
                if o.get("ReturnValue").map(|r| r.field("Handled").truthy()).unwrap_or(false) {
                    return;
                }
            }
        }
    }

    /// widget by name under `root` (first match, depth first)
    pub fn find(&self, root: Id, name: &str) -> Option<Id> {
        let mut d = vec![];
        self.vm.descendants(root, &mut d);
        d.into_iter().find(|&w| self.vm.o(w).name == name)
    }

    /// widget by name anywhere in the viewport
    pub fn find_any(&self, name: &str) -> Option<Id> {
        self.vm.viewport.iter().find_map(|&(r, _)| self.find(r, name))
    }

    /// centre of a widget's last painted rect
    pub fn centre(&self, w: Id) -> Option<[f64; 2]> {
        self.out.rects.get(&w).map(|r| [r[0] + r[2] * 0.5, r[1] + r[3] * 0.5])
    }

    /// is `w` painted (its rect recorded) this frame and on a visible path
    pub fn painted(&self, w: Id) -> bool {
        self.out.rects.contains_key(&w)
    }

    pub fn asset(package: &str, name: &str, class: &str) -> V {
        V::Asset(std::sync::Arc::new(ObjRef { package: package.into(), name: name.into(), class: class.into(), outer: String::new() }))
    }
}

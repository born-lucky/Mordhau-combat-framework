//! The local player's side of the game state, as engine-neutral view models a HUD (mh-runtime's Bevy HUD) renders:
//!   PlayerView  godot/game/mode/mordhau_player_view.gd (MordhauPlayerView): BP_MordhauGameState RepNotify / multicast
//!               handlers -> HUD commands (kill feed, killed-by, match result, the mode's client announcements)
//!   DuelView    godot/game/mode/duel_player_view.gd (DuelPlayerView): BP_DuelPlayerController client logic over the
//!               controller's ReplicatedRoomGame (announcements, countdown, match result, end screen)
//! Texts, durations and thresholds are bytecode literals (Kismet keys kf_* / pc_*, scripts/mode_kismet.py).
//! UNCONFIRMED (as the reference): client replication semantics (on_rep runs when the copied game differs from the
//! last one seen; not the shipped listen-server path).

use crate::event::Ev;
use crate::game_mode::{CtrlId, GameMode, MatchEndInfo, RoomGame};
use crate::kismet::{vf, vs, Kismet};

/// a HUD command (BP_MordhauHUD calls)
#[derive(Clone, Debug, PartialEq)]
pub enum HudCmd {
    /// ShowAnnouncement -> BP_Announcement.Show
    Announce { text: String, subtext: String, duration: f64, src: String },
    /// ShowMatchResult -> BP_Victory / DefeatPopup
    MatchResult { victory: bool, text: String, subtext: String },
    /// SendMessageToKillFeed(Killer, KilledWith, Killed) with GetKillfeedColor of both
    KillFeed { killer: String, with: String, victim: String, killer_color: [f64; 4], victim_color: [f64; 4] },
    /// ShowKilledBy (not drawn by the shipped HUD)
    KilledBy { killer: String },
    Sound { name: String },
    HideMainMenu,
    HideEmoteMenu,
    AllowDrop,
    /// ShowDuelEndscreenDelayed -> HUD.ShowDuelEndScreen
    EndScreen,
}

impl HudCmd {
    /// the reference's command dictionary ({kind, ...}; the golden traces compare it)
    pub fn to_json(&self) -> serde_json::Value {
        use serde_json::json;
        match self {
            HudCmd::Announce { text, subtext, duration, src } => json!({"kind": "announce", "text": text, "subtext": subtext, "duration": duration, "src": src}),
            HudCmd::MatchResult { victory, text, subtext } => json!({"kind": "match_result", "victory": victory, "text": text, "subtext": subtext}),
            HudCmd::KillFeed { killer, with, victim, killer_color, victim_color } => json!({"kind": "kill_feed", "killer": killer,
                "with": with, "victim": victim, "killer_color": killer_color, "victim_color": victim_color}),
            HudCmd::KilledBy { killer } => json!({"kind": "killed_by", "killer": killer}),
            HudCmd::Sound { name } => json!({"kind": "sound", "name": name}),
            HudCmd::HideMainMenu => json!({"kind": "hide_main_menu"}),
            HudCmd::HideEmoteMenu => json!({"kind": "hide_emote_menu"}),
            HudCmd::AllowDrop => json!({"kind": "allow_drop"}),
            HudCmd::EndScreen => json!({"kind": "end_screen"}),
        }
    }
}

fn color(k: &Kismet, key: &str) -> [f64; 4] {
    let a = k.arr(key);
    if a.len() == 4 {
        // a Godot Color (f32 components), as ModeKismet.color builds it
        [vf(&a[0]) as f32 as f64, vf(&a[1]) as f32 as f64, vf(&a[2]) as f32 as f64, vf(&a[3]) as f32 as f64]
    } else {
        [1.0, 0.0, 1.0, 1.0] // magenta: a malformed literal (the reference's Color.MAGENTA)
    }
}

/// MordhauPlayerView
#[derive(Clone, Debug)]
pub struct PlayerView {
    pub me: CtrlId,
    pub out: Vec<HudCmd>,
    end_seen: Option<MatchEndInfo>,
}

impl PlayerView {
    pub fn new(me: CtrlId) -> PlayerView {
        PlayerView { me, out: Vec::new(), end_seen: None }
    }

    pub fn drain(&mut self) -> Vec<HudCmd> {
        std::mem::take(&mut self.out)
    }

    /// feed the mode's events of this tick (before the host drains them for itself)
    pub fn on_events(&mut self, m: &GameMode, evs: &[Ev]) {
        for e in evs {
            match e.kind {
                "kill_notify" => self.kill_notify(m, e),
                "match_end_info" => self.end_info(m, e),
                // the game state's own RepNotify announcements (Skirmish)
                "announce" => self.out.push(announce_of(e)),
                _ => {}
            }
        }
    }

    /// the game state's client tick (BP ReceiveTick on the local machine; Skirmish)
    pub fn tick(&mut self, m: &GameMode) {
        for e in m.client_tick(self.me) {
            self.out.push(announce_of(&e));
        }
    }

    fn end_info(&mut self, m: &GameMode, e: &Ev) {
        let info = MatchEndInfo {
            winner: e.get("winner").map(|a| a.as_s().to_string()).unwrap_or_default(),
            winner_team: e.get("winner_team").map(|a| a.as_i()).unwrap_or(0),
            winner_score: e.get("winner_score").map(|a| a.as_f()).unwrap_or(0.0),
            other_score: e.get("other_score").map(|a| a.as_f()).unwrap_or(0.0),
            draw: e.get("draw").map(|a| a.as_i() != 0).unwrap_or(false),
        };
        if self.end_seen.as_ref() == Some(&info) {
            return;
        }
        self.end_seen = Some(info.clone());
        if let Some(r) = m.match_result_for(self.me, &info) {
            self.out.push(HudCmd::MatchResult { victory: r.victory, text: r.text, subtext: r.subtext });
        }
    }

    /// ReceiveKillNotify (the ReplicatedKillNotify multicast): KilledWith = the weapon's EquipmentName (Flags 0, none ->
    /// "RIP"), "Kick" (1), "Fall" (2); HUD.SendMessageToKillFeed; the local player killed -> ShowKilledBy
    fn kill_notify(&mut self, m: &GameMode, e: &Ev) {
        let k = &m.k;
        let s = |n: &str| e.get(n).map(|a| a.as_s().to_string()).unwrap_or_default();
        let weapon = s("weapon");
        let with = match e.get("flags").map(|a| a.as_i()).unwrap_or(0) {
            0 => {
                if weapon.is_empty() {
                    k.s("kf_rip_text")
                } else {
                    weapon
                }
            }
            1 => k.s("kf_kick_text"),
            2 => k.s("kf_fall_text"),
            _ => String::new(),
        };
        let (kn, vn) = (s("killer"), s("killed"));
        let killer = m.by_name(&kn);
        let victim = m.by_name(&vn);
        self.out.push(HudCmd::KillFeed {
            killer: kn.clone(),
            with,
            victim: vn,
            killer_color: self.kill_feed_color(m, killer),
            victim_color: self.kill_feed_color(m, victim),
        });
        if victim == Some(self.me) && killer != Some(self.me) {
            self.out.push(HudCmd::KilledBy { killer: kn });
        }
    }

    /// BP_MordhauGameState GetKillfeedColor(PlayerState): the local player -> gold; team mode: team 0 red, team 1 blue;
    /// else neutral (linear colours from the bytecode)
    pub fn kill_feed_color(&self, m: &GameMode, c: Option<CtrlId>) -> [f64; 4] {
        if c == Some(self.me) {
            return color(&m.k, "kf_local_color");
        }
        if m.d.state.b_is_team_mode {
            if let Some(c) = c {
                if m.ctrls[c].team == 0 {
                    return color(&m.k, "kf_team0_color");
                }
                if m.ctrls[c].team == 1 {
                    return color(&m.k, "kf_team1_color");
                }
            }
        }
        color(&m.k, "kf_neutral_color")
    }
}

fn announce_of(e: &Ev) -> HudCmd {
    let s = |n: &str| e.get(n).map(|a| a.as_s().to_string()).unwrap_or_default();
    HudCmd::Announce { text: s("text"), subtext: s("subtext"), duration: e.get("duration").map(|a| a.as_f()).unwrap_or(0.0), src: s("src") }
}

/// DuelPlayerView: BP_DuelPlayerController (client logic only)
#[derive(Clone, Debug)]
pub struct DuelView {
    pub c: CtrlId,
    pub last_game: Option<RoomGame>,
    /// BP_DuelPlayerController.Countdown
    pub countdown: i64,
    /// BP_ProfileCustomization.LoadoutSelectionTimer (no CDO default -> 0)
    pub loadout_selection_timer: f64,
    pub out: Vec<HudCmd>,
    end_screen_at: f64,
}

impl DuelView {
    pub fn new(c: CtrlId) -> DuelView {
        DuelView { c, last_game: None, countdown: 0, loadout_selection_timer: 0.0, out: Vec::new(), end_screen_at: -1.0 }
    }

    pub fn drain(&mut self) -> Vec<HudCmd> {
        std::mem::take(&mut self.out)
    }

    fn announce(&mut self, k: &Kismet, key: &str, subtext: String) {
        let v = k.arr(key);
        self.out.push(HudCmd::Announce { text: vs(&v[0]), subtext, duration: vf(&v[1]), src: k.src(key).to_string() });
    }

    /// Format("-{0}-", ArgumentValueInt)
    fn count_text(k: &Kismet, n: i64) -> String {
        k.s("pc_count_format").replace("{0}", &n.to_string())
    }

    /// ReceiveTick (ubergraph 713). `now` = GetTimeSeconds on the client, `server_now` = GameState
    /// GetServerWorldTimeSeconds (one process: the same clock)
    pub fn tick(&mut self, m: &GameMode, now: f64, server_now: f64) {
        let k = m.k.clone();
        let game = m.ctrls[self.c].replicated_room_game;
        if let Some(g) = game {
            if Some(g) != self.last_game {
                self.on_rep(m, g);
                self.last_game = Some(g);
            }
        }
        if self.end_screen_at >= 0.0 && now >= self.end_screen_at {
            self.end_screen_at = -1.0;
            self.out.push(HudCmd::EndScreen);
        }
        let Some(g) = game else { return };
        if !(now > k.f("pc_min_time_s")) {
            return;
        }
        let dd = m.duel_data();
        let st = g.round.stage;
        if st == dd.stage["WaitingForPlayers"] {
            // ShowAnnouncement("Waiting for players", Format("-{0}-", FCeil(FMax(StartTime - ServerTime, 0))), 0.1)
            let v = k.arr("pc_waiting_text");
            let n = (g.round.start_time - server_now).max(0.0).ceil() as i64;
            self.out.push(HudCmd::Announce {
                text: vs(&v[0]),
                duration: vf(&v[1]),
                src: k.src("pc_waiting_text").to_string(),
                subtext: Self::count_text(&k, n),
            });
        } else if st == dd.stage["WaitingToStart"] {
            // LoadoutSelectionTimer = FMax(timer, StartTime - ServerTime + now + 5): the 5 s RoundStart that follows
            self.loadout_selection_timer = self.loadout_selection_timer.max(g.round.start_time - server_now + now + k.f("pc_loadout_extra_s"));
            self.update_countdown(&k, self.loadout_selection_timer, now);
        } else if st == dd.stage["RoundStart"] {
            self.loadout_selection_timer = (g.round.start_time - server_now + now).max(self.loadout_selection_timer);
            self.update_countdown(&k, self.loadout_selection_timer, now);
        } else if st == dd.stage["RoundPlay"] {
            self.countdown = 0;
        }
    }

    /// UpdateCountdown(StartTime): Countdown = Max(FCeil(StartTime - now), 0); on change: <= 10 PlaySound2D(UI_ClockTickCue);
    /// <= 5 "Get ready!" else "Match starting", subtext "-N-", 1.2 s
    fn update_countdown(&mut self, k: &Kismet, start_time: f64, now: f64) {
        let n = ((start_time - now).ceil() as i64).max(0);
        if n == self.countdown {
            return;
        }
        self.countdown = n;
        if n <= k.i("pc_tick_sound_at") {
            self.out.push(HudCmd::Sound { name: "UI_ClockTickCue".into() });
        }
        let t = Self::count_text(k, n);
        if n <= k.i("pc_get_ready_at") {
            self.announce(k, "pc_get_ready", t);
        } else {
            self.announce(k, "pc_match_starting", t);
        }
    }

    /// OnRep_ReplicatedRoomGame: RoundStart: hide the emote menu. RoundPlay: "Fight!" 2 s, HideMainMenu,
    /// PlaySound2D(EnemyCapped). RoundEnd: Winner == my team -> (a team has 5 wins ? ShowMatchResult(true, "victory") :
    /// "Round won" 3 s) else (5 wins ? ShowMatchResult(false, "defeat") : "Round lost" 3 s); then Stage == RoundPlay ->
    /// pawn bAllowDrop = true
    fn on_rep(&mut self, m: &GameMode, g: RoomGame) {
        let k = m.k.clone();
        let dd = m.duel_data();
        let st = g.round.stage;
        if st == dd.stage["RoundStart"] {
            self.out.push(HudCmd::HideEmoteMenu);
        } else if st == dd.stage["RoundPlay"] {
            self.announce(&k, "pc_fight", String::new());
            self.out.push(HudCmd::HideMainMenu);
            self.out.push(HudCmd::Sound { name: "EnemyCapped".into() });
        } else if st == dd.stage["RoundEnd"] {
            let over_w = k.i("pc_match_over_wins");
            let over = g.team1_wins == over_w || g.team2_wins == over_w;
            let won = g.round.winner == (m.ctrls[self.c].team & 0xff);
            if over {
                let key = if won { "pc_victory" } else { "pc_defeat" };
                self.out.push(HudCmd::MatchResult { victory: won, text: k.s(key), subtext: String::new() });
            } else {
                self.announce(&k, if won { "pc_round_won" } else { "pc_round_lost" }, String::new());
            }
        }
        if st == k.i("pc_drop_stage") {
            self.out.push(HudCmd::AllowDrop);
        }
    }

    /// OnRep_NewMMR -> OnReceivedMMR + ShowDuelEndscreenDelayed (Delay 6.5 s -> HUD.ShowDuelEndScreen)
    pub fn on_new_mmr(&mut self, k: &Kismet, now: f64) {
        self.end_screen_at = now + k.f("pc_endscreen_delay_s");
    }

    /// BP_DuelGameState GetScoreboardTeamObjectiveValue(Team): the local controller's ReplicatedRoomGame wins
    pub fn team_wins(&self, m: &GameMode, team: i64) -> i64 {
        match m.ctrls[self.c].replicated_room_game {
            None => 0,
            Some(g) => {
                if team == 0 {
                    g.team1_wins
                } else {
                    g.team2_wins
                }
            }
        }
    }
}

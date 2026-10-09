//! HUD view models, engine-agnostic: ports of godot/game/ui/status_bar_model.gd (BP_StatusBar bytecode),
//! announcement_model.gd (BP_Announcement / BP_Victory|DefeatPopup timing) and kill_feed_model.gd (BP_KillFeed), with
//! their citations. Constants: mode_kismet.json (Kismet keys sb_* / kf_* / victory_* / defeat_* / hud_*) and the widget
//! CDOs (mh-assets UmgPackage::defaults).

use mh_mode::Kismet;
use mh_mode::kismet::vf;

/// FMath::FInterpTo rva=0x18aae60: speed <= 0 -> target; dist^2 < 1e-8 (rdata 0x144014a88) -> target;
/// current + dist * clamp(dt * speed, 0, 1)
pub fn finterp_to(cur: f64, target: f64, dt: f64, speed: f64) -> f64 {
    if speed <= 0.0 {
        return target;
    }
    let d = target - cur;
    if d * d < 1e-8 {
        return target;
    }
    cur + d * (dt * speed).clamp(0.0, 1.0)
}

/// FMath::FInterpConstantTo rva=0x18aae00
pub fn finterp_constant_to(cur: f64, target: f64, dt: f64, speed: f64) -> f64 {
    let d = target - cur;
    if d * d < 1e-8 {
        return target;
    }
    let step = dt * speed;
    cur + d.clamp(-step, step)
}

/// UKismetMathLibrary::MapRangeUnclamped rva=0x31887a0
pub fn map_range_unclamped(v: f64, in_a: f64, in_b: f64, out_a: f64, out_b: f64) -> f64 {
    let r = in_b - in_a;
    let pct = if r.abs() > 1e-8 { (v - in_a) / r } else if v >= in_b { 1.0 } else { 0.0 };
    out_a + pct * (out_b - out_a)
}

/// BP_StatusBar numbers (status_bar_model.gd)
#[derive(Clone, Debug)]
pub struct StatusBarModel {
    k: Kismet,
    pub observed_health: i64,
    pub observed_stamina: i64,
    last_health: i64,
    last_stamina: i64,
    observed_id: Option<u64>,
    pub displayed_health: f64,
    pub displayed_stamina: f64,
    pub delayed_health: f64,
    pub delayed_stamina: f64,
    pub delayed_health_wait: f64,
    pub delayed_stamina_wait: f64,
    pub delayed_health_target: f64,
    pub delayed_stamina_target: f64,
    pub health_pulse: f64,
    pub stamina_pulse: f64,
}

impl StatusBarModel {
    /// `cdo` = BP_StatusBar's class defaults (DisplayedHealth ... DelayedStaminaTarget, all 100 in the package)
    pub fn new(k: Kismet, cdo: Option<&serde_json::Value>) -> StatusBarModel {
        let g = |key: &str| cdo.and_then(|c| c.get(key)).and_then(|v| v.as_f64()).unwrap_or(100.0);
        StatusBarModel {
            observed_health: 0,
            observed_stamina: 0,
            last_health: 0,
            last_stamina: 0,
            observed_id: None,
            displayed_health: g("DisplayedHealth"),
            displayed_stamina: g("DisplayedStamina"),
            delayed_health: g("DelayedHealth"),
            delayed_stamina: g("DelayedStamina"),
            delayed_health_wait: g("DelayedHealthWait"),
            delayed_stamina_wait: g("DelayedStaminaWait"),
            delayed_health_target: g("DelayedHealthTarget"),
            delayed_stamina_target: g("DelayedStaminaTarget"),
            health_pulse: 0.0,
            stamina_pulse: 0.0,
            k,
        }
    }

    fn set_observed(&mut self, target: Option<u64>, health: i64, stamina: i64) {
        if target.is_some() {
            self.observed_stamina = stamina;
            self.observed_health = health;
        } else {
            self.observed_stamina = 0;
            self.observed_health = 0;
        }
        if target.is_some() && target == self.observed_id {
            if self.observed_health < self.last_health {
                self.delayed_health_wait = self.k.f("sb_wait_on_drop");
                self.delayed_health_target = self.last_health as f64;
            }
            if self.observed_stamina < self.last_stamina {
                self.delayed_stamina_wait = self.k.f("sb_wait_on_drop");
                self.delayed_stamina_target = self.last_stamina as f64;
            }
        } else {
            self.delayed_health = self.observed_health as f64;
            self.displayed_health = self.observed_health as f64;
            self.delayed_stamina = self.observed_stamina as f64;
            self.displayed_stamina = self.observed_stamina as f64;
            self.delayed_health_wait = 0.0;
            self.delayed_stamina_wait = 0.0;
        }
        self.observed_id = target;
        self.last_health = self.observed_health;
        self.last_stamina = self.observed_stamina;
    }

    /// one HUD tick for the view target (None = no target) and its health / stamina bytes
    pub fn tick(&mut self, dt: f64, target: Option<u64>, health: i64, stamina: i64) {
        self.set_observed(target, health, stamina);
        let k = &self.k;
        let oh = self.observed_health as f64;
        self.delayed_health_wait = finterp_constant_to(self.delayed_health_wait, 0.0, dt, k.f("sb_wait_decay"));
        self.displayed_health = if oh > self.displayed_health { finterp_to(self.displayed_health, oh, dt, k.f("sb_health_regen_speed")) } else { oh };
        if self.delayed_health > self.displayed_health {
            self.delayed_health = if self.delayed_health_wait == 0.0 {
                finterp_to(self.delayed_health, oh, dt, k.f("sb_health_delayed_speed"))
            } else {
                finterp_to(self.delayed_health, self.delayed_health_target, dt, k.f("sb_health_target_speed"))
            };
        } else {
            self.delayed_health = self.displayed_health;
        }
        let os = self.observed_stamina as f64;
        self.delayed_stamina_wait = finterp_constant_to(self.delayed_stamina_wait, 0.0, dt, k.f("sb_stam_wait_decay"));
        self.displayed_stamina = if os > self.displayed_stamina { finterp_to(self.displayed_stamina, os, dt, k.f("sb_stam_regen_speed")) } else { os };
        if self.delayed_stamina > self.displayed_stamina {
            self.delayed_stamina = if self.delayed_stamina_wait == 0.0 {
                finterp_to(self.delayed_stamina, self.displayed_stamina, dt, k.f("sb_stam_delayed_speed"))
            } else {
                finterp_to(self.delayed_stamina, self.delayed_stamina_target, dt, k.f("sb_stam_target_speed"))
            };
        } else {
            self.delayed_stamina = self.displayed_stamina;
        }
        let pr: Vec<f64> = k.arr("sb_pulse_range").iter().map(vf).collect();
        self.health_pulse = if self.displayed_health <= pr[0] { map_range_unclamped(self.displayed_health, pr[0], pr[1], pr[2], pr[3]) } else { 0.0 };
        self.stamina_pulse = if self.displayed_stamina <= pr[0] { map_range_unclamped(self.displayed_stamina, pr[0], pr[1], pr[2], pr[3]) } else { 0.0 };
    }

    pub fn health_percent(&self) -> f64 {
        self.displayed_health / self.k.f("sb_percent_div")
    }
    pub fn stamina_percent(&self) -> f64 {
        self.displayed_stamina / self.k.f("sb_percent_div")
    }
    pub fn delayed_health_percent(&self) -> f64 {
        self.delayed_health / self.k.f("sb_percent_div")
    }
    pub fn delayed_stamina_percent(&self) -> f64 {
        self.delayed_stamina / self.k.f("sb_percent_div")
    }
    /// getHealthPercentageText: Conv_IntToText(FCeil(DisplayedHealth))
    pub fn health_text(&self) -> String {
        (self.displayed_health.ceil() as i64).to_string()
    }
    pub fn stamina_text(&self) -> String {
        (self.displayed_stamina.ceil() as i64).to_string()
    }
}

/// UMovieScene default TickResolution 60000/s (UE 4.26; announcement_model.gd, UNCONFIRMED there)
pub const TICK_RESOLUTION: f64 = 60000.0;

/// BP_Announcement (announcement_model.gd AnnouncementModel)
#[derive(Clone, Debug, Default)]
pub struct AnnouncementModel {
    pub text: String,
    pub subtext: String,
    pub showing: bool,
    timer_at: f64,
    exit_until: f64,
    entry_from: f64,
    pub entry_len: f64,
    pub exit_len: f64,
    now: f64,
}

impl AnnouncementModel {
    /// entry / exit = the "Entry Anim" / "Exit Anim" MovieScene last ticks / TICK_RESOLUTION
    pub fn new(entry_len: f64, exit_len: f64) -> Self {
        AnnouncementModel { timer_at: -1.0, exit_until: -1.0, entry_from: -1.0, entry_len, exit_len, ..Default::default() }
    }
    pub fn show(&mut self, t: &str, sub: &str, duration: f64) {
        self.exit_until = -1.0;
        self.text = t.into();
        self.subtext = sub.into();
        self.timer_at = self.now + duration;
        if !self.showing {
            self.showing = true;
            self.entry_from = self.now;
        }
    }
    pub fn tick(&mut self, dt: f64) {
        self.now += dt;
        if self.timer_at >= 0.0 && self.now >= self.timer_at {
            self.timer_at = -1.0;
            self.exit_until = self.now + self.exit_len;
        }
        if self.exit_until >= 0.0 && self.now >= self.exit_until {
            self.exit_until = -1.0;
            self.showing = false;
        }
    }
    /// view opacity (UNCONFIRMED stand-in for the Entry/Exit Anim tracks)
    pub fn opacity(&self) -> f64 {
        if !self.showing {
            return 0.0;
        }
        if self.exit_until >= 0.0 && self.exit_len > 0.0 {
            return ((self.exit_until - self.now) / self.exit_len).clamp(0.0, 1.0);
        }
        if self.entry_len > 0.0 {
            return ((self.now - self.entry_from) / self.entry_len).clamp(0.0, 1.0);
        }
        1.0
    }
    pub fn subtext_visible(&self) -> bool {
        !self.subtext.is_empty()
    }
}

/// BP_VictoryPopup / BP_DefeatPopup (announcement_model.gd ResultPopup)
#[derive(Clone, Debug, Default)]
pub struct ResultPopup {
    pub victory: bool,
    pub text: String,
    pub subtext: String,
    pub showing: bool,
    start_at: f64,
    hide_at: f64,
    exit_until: f64,
    entry_len: f64,
    exit_len: f64,
    duration: f64,
    exit_rate: f64,
    shown_at: f64,
    now: f64,
}

impl ResultPopup {
    pub fn new() -> Self {
        ResultPopup { start_at: -1.0, hide_at: -1.0, exit_until: -1.0, exit_rate: 1.0, ..Default::default() }
    }
    /// `anim(victory, scene)` = a popup MovieScene's length in seconds (EntryAnim / the exit anim the kismet names)
    pub fn show(&mut self, k: &Kismet, anim: &dyn Fn(bool, &str) -> f64, victory: bool, t: &str, sub: &str) {
        self.victory = victory;
        let ex = k.arr(if victory { "victory_exit" } else { "defeat_exit" }).clone();
        self.entry_len = anim(victory, "EntryAnim");
        self.exit_len = anim(victory, ex.first().and_then(|v| v.as_str()).unwrap_or("ExitAnim"));
        self.exit_rate = ex.get(2).map(vf).unwrap_or(1.0);
        self.text = t.into();
        self.subtext = sub.into();
        self.duration = k.f("hud_result_duration_s");
        self.start_at = self.now + k.f(if victory { "victory_delay_s" } else { "defeat_delay_s" });
        self.hide_at = -1.0;
        self.exit_until = -1.0;
    }
    pub fn tick(&mut self, dt: f64) {
        self.now += dt;
        if self.start_at >= 0.0 && self.now >= self.start_at {
            self.start_at = -1.0;
            self.showing = true;
            self.shown_at = self.now;
            self.hide_at = self.now + self.duration;
        }
        if self.hide_at >= 0.0 && self.now >= self.hide_at {
            self.hide_at = -1.0;
            self.exit_until = self.now + self.exit_len / self.exit_rate;
        }
        if self.exit_until >= 0.0 && self.now >= self.exit_until {
            self.exit_until = -1.0;
            self.showing = false;
        }
    }
    pub fn opacity(&self) -> f64 {
        if !self.showing {
            return 0.0;
        }
        if self.exit_until >= 0.0 && self.exit_len > 0.0 {
            return ((self.exit_until - self.now) / (self.exit_len / self.exit_rate)).clamp(0.0, 1.0);
        }
        if self.entry_len > 0.0 {
            return ((self.now - self.shown_at) / self.entry_len).clamp(0.0, 1.0);
        }
        1.0
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct FeedEntry {
    pub killer: String,
    pub with: String,
    pub victim: String,
    pub killer_color: [f64; 4],
    pub victim_color: [f64; 4],
}

/// BP_KillFeed (kill_feed_model.gd)
#[derive(Clone, Debug, Default)]
pub struct KillFeedModel {
    pub entries: Vec<FeedEntry>,
    pub visible: bool,
    hide_at: f64,
    now: f64,
    pub max_entries: usize,
    pub time_to_disappear: f64,
}

impl KillFeedModel {
    /// `cdo` = BP_KillFeed's class defaults (MaxEntries, TimeForKillFeedToDisappear)
    pub fn new(cdo: Option<&serde_json::Value>) -> Self {
        let g = |k: &str| cdo.and_then(|c| c.get(k)).and_then(|v| v.as_f64());
        KillFeedModel { hide_at: -1.0, max_entries: g("MaxEntries").unwrap_or(8.0) as usize, time_to_disappear: g("TimeForKillFeedToDisappear").unwrap_or(5.0), ..Default::default() }
    }
    pub fn add(&mut self, killer: &str, with: &str, victim: &str, kc: [f64; 4], vc: [f64; 4]) {
        if self.entries.len() >= self.max_entries {
            self.entries.remove(0);
        }
        self.entries.push(FeedEntry {
            killer: if killer == victim { String::new() } else { killer.into() },
            with: with.into(),
            victim: victim.into(),
            killer_color: kc,
            victim_color: vc,
        });
        self.visible = true;
        self.hide_at = self.now + self.time_to_disappear;
    }
    pub fn tick(&mut self, dt: f64) {
        self.now += dt;
        if self.hide_at >= 0.0 && self.now >= self.hide_at {
            self.hide_at = -1.0;
            self.entries.clear();
            self.visible = false;
        }
    }
}

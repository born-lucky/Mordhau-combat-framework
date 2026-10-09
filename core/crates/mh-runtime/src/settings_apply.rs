//! settings_apply.rs (rust-ui r3 hook): the Video / Audio settings of UMordhauGameUserSettings take effect in the
//! running game, on start and whenever the menu applies them (mh_ui::UiSettingsApplied), as in the exe:
//! - UGameUserSettings::ApplyResolutionSettings (UE 4.26): RequestResolutionChange(ResolutionSizeX/Y, FullscreenMode)
//!   -> the window's size and mode. EWindowMode: 0 Fullscreen (exclusive), 1 WindowedFullscreen, 2 Windowed.
//! - UGameUserSettings::ApplyNonResolutionSettings: r.VSync = bUseVSync -> the swap chain's present mode;
//!   t.MaxFPS = FrameRateLimit (0 = unlimited) -> frame pacing.
//! - UMordhauGameUserSettings::ApplyNonResolutionSettings rva=0x1582870 / ApplyAudioVolumes rva=0x15825d0 ->
//!   UMordhauUtilityLibrary::SetSoundMixVolume rva=0x163e160 for GlobalMaster (Master), Effects, Voice,
//!   Instruments, Music (mh-assets ClassTree::apply_mix_volume holds the class walk).
//! The values come from the rewrite's own GameUserSettings.ini (mh_ui::settings::config_dir), which the menu has
//! already written when UiSettingsApplied arrives; a missing key keeps the UMordhauGameUserSettings::SetToDefaults /
//! UGameUserSettings defaults (mh-ui settings.rs table).

use bevy::prelude::*;
use bevy::window::{MonitorSelection, PresentMode, PrimaryWindow, VideoModeSelection, WindowMode};
use std::collections::HashMap;

#[derive(Resource, Clone, Debug, PartialEq)]
pub struct VideoAudioSettings {
    pub resolution: (u32, u32),
    /// EWindowMode
    pub fullscreen_mode: i32,
    pub vsync: bool,
    /// t.MaxFPS; 0 = unlimited
    pub frame_rate_limit: f32,
    /// SetSoundMixVolume order: GlobalMaster, Effects, Music, Voice, Instruments
    pub volumes: [f64; 5],
}

impl VideoAudioSettings {
    pub fn load() -> VideoAudioSettings {
        let p = mh_ui::settings::config_dir().join("GameUserSettings.ini");
        let m = std::fs::read_to_string(&p).map(|t| crate::usersettings::parse(&t)).unwrap_or_default();
        Self::from_map(&m)
    }

    pub fn from_map(m: &HashMap<String, String>) -> VideoAudioSettings {
        let f = |k: &str, d: f64| m.get(k).and_then(|v| v.trim().parse::<f64>().ok()).unwrap_or(d);
        let b = |k: &str, d: bool| m.get(k).map(|v| v.trim().eq_ignore_ascii_case("true") || v.trim() == "1").unwrap_or(d);
        VideoAudioSettings {
            // a fresh config has no resolution (GetDefaultResolution 0x0): the window keeps the desktop size
            resolution: (f("ResolutionSizeX", 0.0) as u32, f("ResolutionSizeY", 0.0) as u32),
            // the defaults mh-ui's settings table uses (SetToDefaults): FullscreenMode 0, no vsync, FrameRateLimit 60,
            // the values the real game's own fresh GameUserSettings.ini holds
            fullscreen_mode: f("FullscreenMode", 0.0) as i32,
            vsync: b("bUseVSync", false),
            frame_rate_limit: f("FrameRateLimit", 60.0) as f32,
            // UMordhauGameUserSettings::SetToDefaults rva=0x15a7c90: Master / Effects / Voice / Instruments 1.0, Music 0.5
            volumes: [f("MasterVolume", 1.0), f("EffectsVolume", 1.0), f("MusicVolume", 0.5), f("VoiceVolume", 1.0), f("InstrumentsVolume", 1.0)],
        }
    }
}

pub struct SettingsApplyPlugin;

impl Plugin for SettingsApplyPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(VideoAudioSettings::load())
            .init_resource::<Applied>()
            .add_systems(Update, (reload, apply_window, apply_volumes).chain())
            .add_systems(Last, frame_limit);
    }
}

#[derive(Resource, Default)]
struct Applied {
    window: Option<VideoAudioSettings>,
    /// (volumes, number of loaded sound classes) last pushed into the class tree
    audio: Option<([f64; 5], usize)>,
}

fn reload(mut ev: MessageReader<mh_ui::UiSettingsApplied>, mut s: ResMut<VideoAudioSettings>) {
    if ev.read().any(|e| !e.input) {
        let n = VideoAudioSettings::load();
        if *s != n {
            info!("settings: video / audio re-applied {n:?}");
            *s = n;
        }
    }
}

fn apply_window(s: Res<VideoAudioSettings>, mut applied: ResMut<Applied>, mut win: Query<&mut Window, With<PrimaryWindow>>) {
    let Ok(mut w) = win.single_mut() else { return };
    if applied.window.as_ref() == Some(&*s) {
        return;
    }
    let (x, y) = s.resolution;
    w.mode = match s.fullscreen_mode {
        0 => WindowMode::Fullscreen(MonitorSelection::Current, VideoModeSelection::Current),
        1 => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
        _ => WindowMode::Windowed,
    };
    if x > 0 && y > 0 {
        w.resolution.set_physical_resolution(x, y);
    }
    w.present_mode = if s.vsync { PresentMode::AutoVsync } else { PresentMode::AutoNoVsync };
    applied.window = Some(s.clone());
}

fn apply_volumes(s: Res<VideoAudioSettings>, mut applied: ResMut<Applied>, st: Option<NonSendMut<mh_audio::AudioState>>) {
    let Some(mut st) = st else { return };
    let n = st.classes.tree.classes.len();
    if applied.audio == Some((s.volumes, n)) {
        return;
    }
    // SetSoundMixVolume rva=0x163e160: 0 GlobalMaster (x CMasterVolumeMultiplier 1.0), 1 Effects, 2 Music, 3 Voice,
    // 4 Instruments; re-pushed when classes load later (the exe walks every loaded USoundClass)
    for (name, v) in ["GlobalMaster", "Effects", "Music", "Voice", "Instruments"].iter().zip(s.volumes) {
        st.classes.tree.apply_mix_volume(name, v);
    }
    st.classes.effective = st.classes.tree.effective(&[]);
    applied.audio = Some((s.volumes, n));
}

/// t.MaxFPS (FrameRateLimit; UEngine::GetMaxTickRate): the frame waits out the rest of 1 / limit
/// (windowed only: headless / offscreen runs keep their own --fps pacing)
fn frame_limit(s: Res<VideoAudioSettings>, win: Query<(), With<PrimaryWindow>>, mut last: Local<Option<std::time::Instant>>) {
    let now = std::time::Instant::now();
    if s.frame_rate_limit > 0.0 && !win.is_empty() {
        if let Some(prev) = *last {
            let target = std::time::Duration::from_secs_f64(1.0 / s.frame_rate_limit as f64);
            let spent = now - prev;
            if spent < target {
                std::thread::sleep(target - spent);
            }
        }
    }
    *last = Some(std::time::Instant::now());
}

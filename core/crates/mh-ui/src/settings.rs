//! UMordhauGameUserSettings for the settings screens (BP_GameSettings / BP_VideoSettings / BP_AudioSettings /
//! BP_ControlsSettings call it through GetMordhauGameUserSettings()). Fields, defaults, accessors and slider limits
//! are the exe port in godot/game/ui/user_settings.gd (UMordhauGameUserSettings.cpp decomp), tables generated from it;
//! values are loaded from and saved to the user's real GameUserSettings.ini the way UObject::LoadConfig / SaveConfig
//! do for a config=GameUserSettings class: section [/Script/Mordhau.MordhauGameUserSettings], one `Key=Value` line per
//! UPROPERTY (FProperty::ExportText formats: floats "%f", bools True/False, enums / names bare).
//!   Config dir: $MH_CONFIG_DIR, else %LOCALAPPDATA%\MordhauRewrite\Saved\Config\WindowsClient: the rewrite's OWN
//!   generated-config dir (user directive 2026-10-05: never the Steam install's %LOCALAPPDATA%\Mordhau\Saved, so a
//!   first run is a fresh install: SetToDefaults / the UMordhauInput ctor values; fidelity-audit r7).
//!   SaveSettings -> SaveConfig: every known key of the section rewritten in place, other lines and sections kept
//!   (UNCONFIRMED: UE rewrites the whole file from its FConfigFile; the in-place rewrite keeps the user's other
//!   settings byte-identical).
//!   Scalability (Get/Set<Group>Quality) is the [ScalabilityGroups] section's sg.<Group>Quality (UE 4.26
//!   Scalability::SaveState).

use crate::model::*;
use crate::vm::Vm;
use std::path::PathBuf;

pub const SECTION: &str = "[/Script/Mordhau.MordhauGameUserSettings]";

/// SetToDefaults 0x15a7c90 (godot/game/ui/user_settings.gd set_to_defaults): (ini key, default)
pub const DEFAULTS: &[(&str, fn() -> V)] = &[
    ("Language", || V::Str("English".into())),
    ("Gore", || V::Int(2)),
    ("ProfanityFilter", || V::Int(1)),
    ("ThirdPersonDeathCamera", || V::Int(0)),
    ("CharacterCloth", || V::Int(2)),
    ("FriendlyMarkers", || V::Int(0)),
    ("NoTeamColorsOnGear", || V::Int(0)),
    ("Headbob", || V::Float(1.0)),
    ("MovementHeadbob", || V::Float(1.0)),
    ("CombatHeadbob", || V::Float(1.0)),
    ("MaxRagdolls", || V::Int(10)),
    ("RagdollStayTime", || V::Float(30.0)),
    ("MouseSmoothing", || V::Float(0.0)),
    ("DrawTracers", || V::Int(0)),
    ("DrawTracersStayTime", || V::Float(2.0)),
    ("ForceFeedback", || V::Bool(false)),
    ("bCrossplayEnabled", || V::Bool(true)),
    ("RangedSensitivity", || V::Float(0.35)),
    ("HideHUD", || V::Int(0)),
    ("HideDefaultLoadouts", || V::Int(0)),
    ("ShowServerInScoreboard", || V::Int(1)),
    ("CrosshairType", || V::Int(0)),
    ("ShowKilledBy", || V::Int(1)),
    ("ShowStatusBar", || V::Int(1)),
    ("ShowTargetInfo", || V::Int(1)),
    ("ShowSpawnInfo", || V::Int(1)),
    ("ShowChatBox", || V::Int(1)),
    ("ShowEmotesMenu", || V::Int(1)),
    ("ShowEquipment", || V::Int(1)),
    ("ShowAmmo", || V::Int(1)),
    ("ShowAnnouncements", || V::Int(1)),
    ("ShowTips", || V::Int(1)),
    ("ShowObjectives", || V::Int(1)),
    ("ShowHitMarker", || V::Int(1)),
    ("ShowScoreFeed", || V::Int(1)),
    ("ShowCombatHints", || V::Int(1)),
    ("ShowKillFeed", || V::Int(1)),
    ("ShowObservedDelay", || V::Int(0)),
    ("ScreenPercentage", || V::Float(100.0)),
    ("FullscreenMode", || V::Int(0)),
    ("FrameRateLimit", || V::Float(60.0)),
    ("bUseVSync", || V::Bool(false)),
    ("FieldOfView", || V::Float(78.0)),
    ("CameraDistance", || V::Float(0.0)),
    ("Gamma", || V::Float(1.0)),
    ("AntiAliasing", || V::Int(2)),
    ("IndirectCapsuleShadows", || V::Int(1)),
    ("CharacterFidelity", || V::Int(2)),
    ("RagdollFidelity", || V::Int(1)),
    ("Bloom", || V::Float(1.0)),
    ("MotionBlur", || V::Float(0.09)),
    ("ScreenSpaceReflections", || V::Int(1)),
    ("AmbientOcclusion", || V::Int(1)),
    ("LensFlares", || V::Int(1)),
    ("PlatformSpecific", || V::Bool(true)),
    ("MasterVolume", || V::Float(1.0)),
    ("EffectsVolume", || V::Float(1.0)),
    ("MusicVolume", || V::Float(0.5)),
    ("VideoVolume", || V::Float(1.0)),
    ("VoiceVolume", || V::Float(1.0)),
    ("InstrumentsVolume", || V::Float(1.0)),
    ("CasualMatchmakingRegion", || V::Int(9)),
    ("CasualMatchmakingGameModes", || V::Str("Invasion & Frontline".into())),
    ("RankedMatchmakingRegion", || V::Int(9)),
    ("RankedMatchmakingGameModes", || V::Str("Teamfight".into())),
    ("bServerBrowserIsOfficial", || V::Bool(false)),
    ("bServerBrowserConsoleServer", || V::Bool(false)),
    ("bServerBrowserNotFull", || V::Bool(true)),
    ("bServerBrowserHasPlayers", || V::Bool(false)),
    ("bServerBrowserNoPassword", || V::Bool(true)),
    ("ServerBrowserGameMode", || V::Str("All".into())),
    ("ServerBrowserMaxPing", || V::Int(100)),
    ("bIsPSNLockEnabled", || V::Bool(false)),
];
/// GETTERS: (UFUNCTION, ini key of the field it loads)
pub const GETTERS: &[(&str, &str)] = &[
    ("GetAmbientOcclusion", "AmbientOcclusion"), // `GetAmbientOcclusion` 0x1594ca0
    ("GetAntiAliasing", "AntiAliasing"), // `GetAntiAliasing` 0x1594cd0
    ("GetBloom", "Bloom"), // `GetBloom` 0x15951a0
    ("GetCameraDistance", "CameraDistance"), // `GetCameraDistance` 0x15951b0
    ("GetCasualMatchmakingRegion", "CasualMatchmakingRegion"), // `GetCasualMatchmakingRegion` 0x15952a0
    ("GetCharacterCloth", "CharacterCloth"), // `GetCharacterCloth` 0x15952d0
    ("GetCharacterFidelity", "CharacterFidelity"), // `GetCharacterFidelity` 0x15952e0
    ("GetCombatHeadbob", "MovementHeadbob"), // `GetCombatHeadbob` 0x15952f0
    ("GetCrosshairType", "CrosshairType"), // `GetCrosshairType` 0x1595330
    ("GetCrossplayEnabled", "bCrossplayEnabled"), // `GetCrossplayEnabled` 0x1595340
    ("GetDrawTracers", "DrawTracers"), // `GetDrawTracers` 0x1595bb0
    ("GetEffectsVolume", "EffectsVolume"), // `GetEffectsVolume` 0x1595bc0
    ("GetFieldOfView", "FieldOfView"), // `GetFieldOfView` 0x1595de0
    ("GetForceFeedbackEnabled", "ForceFeedback"), // `GetForceFeedbackEnabled` 0x1595e10
    ("GetFriendlyMarkers", "FriendlyMarkers"), // `GetFriendlyMarkers` 0x1595e40
    ("GetGamma", "Gamma"), // `GetGamma` 0x15961c0
    ("GetGore", "Gore"), // `GetGore` 0x15961d0
    ("GetHeadbob", "Headbob"), // `GetHeadbob` 0x14fc520
    ("GetHideDefaultLoadouts", "HideDefaultLoadouts"), // `GetHideDefaultLoadouts` 0x15961f0
    ("GetHideHUD", "HideHUD"), // `GetHideHUD` 0x1596200
    ("GetIndirectCapsuleShadows", "IndirectCapsuleShadows"), // `GetIndirectCapsuleShadows` 0x1596210
    ("GetInstrumentsVolume", "InstrumentsVolume"), // `GetInstrumentsVolume` 0x1596220
    ("GetLensFlares", "LensFlares"), // `GetLensFlares` 0xeaf0f0
    ("GetMasterVolume", "MasterVolume"), // `GetMasterVolume` 0x1596c80
    ("GetMaxRagdolls", "MaxRagdolls"), // `GetMaxRagdolls` 0x1596c90
    ("GetMotionBlur", "MotionBlur"), // `GetMotionBlur` 0x1596cb0
    ("GetMouseSmoothing", "MouseSmoothing"), // `GetMouseSmoothing` 0x1596d00
    ("GetMovementHeadbob", "MovementHeadbob"), // `GetMovementHeadbob` 0x15952f0
    ("GetMusicVolume", "MusicVolume"), // `GetMusicVolume` 0x1596d50
    ("GetNoTeamColorsOnGear", "NoTeamColorsOnGear"), // `GetNoTeamColorsOnGear` 0x1597cc0
    ("GetNvidiaReflex", "NvidiaReflex"), // `GetNvidiaReflex` 0x1597cd0
    ("GetPlatformSpecific", "PlatformSpecific"), // `GetPlatformSpecific` 0x1597d10
    ("GetProfanityFilter", "ProfanityFilter"), // `GetProfanityFilter` 0x1598ac0
    ("GetPSNLockEnabledValue", "bIsPSNLockEnabled"), // `GetPSNLockEnabledValue` 0xeaf720
    ("GetRagdollFidelity", "RagdollFidelity"), // `GetRagdollFidelity` 0x1598ad0
    ("GetRagdollStayTime", "RagdollStayTime"), // `GetRagdollStayTime` 0x1598ae0
    ("GetRangedSensitivity", "RangedSensitivity"), // `GetRangedSensitivity` 0x1598b00
    ("GetRankedMatchmakingRegion", "RankedMatchmakingRegion"), // `GetRankedMatchmakingRegion` 0x1598bf0
    ("GetScreenPercentage", "ScreenPercentage"), // `GetScreenPercentage` 0x1598d30
    ("GetScreenSpaceReflections", "ScreenSpaceReflections"), // `GetScreenSpaceReflections` 0x1598d60
    ("GetServerBrowserHasPlayers", "bServerBrowserHasPlayers"), // `GetServerBrowserHasPlayers` 0x15993c0
    ("GetServerBrowserIsConsoleServer", "bServerBrowserConsoleServer"), // `GetServerBrowserIsConsoleServer` 0x15993d0
    ("GetServerBrowserIsOfficial", "bServerBrowserIsOfficial"), // `GetServerBrowserIsOfficial` 0x15993e0
    ("GetServerBrowserMaxPing", "ServerBrowserMaxPing"), // `GetServerBrowserMaxPing` 0x15993f0
    ("GetServerBrowserNoPassword", "bServerBrowserNoPassword"), // `GetServerBrowserNoPassword` 0x1599400
    ("GetServerBrowserNotFull", "bServerBrowserNotFull"), // `GetServerBrowserNotFull` 0x1599410
    ("GetShowAmmo", "ShowAmmo"), // `GetShowAmmo` 0x1599550
    ("GetShowAnnouncements", "ShowAnnouncements"), // `GetShowAnnouncements` 0x1599560
    ("GetShowChatBox", "ShowChatBox"), // `GetShowChatBox` 0x1599570
    ("GetShowCombatHints", "ShowCombatHints"), // `GetShowCombatHints` 0x1599580
    ("GetShowEmotesMenu", "ShowEmotesMenu"), // `GetShowEmotesMenu` 0x1599590
    ("GetShowEquipment", "ShowEquipment"), // `GetShowEquipment` 0x15995a0
    ("GetShowHitMarker", "ShowHitMarker"), // `GetShowHitMarker` 0x15995b0
    ("GetShowKillFeed", "ShowKillFeed"), // `GetShowKillFeed` 0x15995c0
    ("GetShowKilledBy", "ShowKilledBy"), // `GetShowKilledBy` 0x15995d0
    ("GetShowObjectives", "ShowObjectives"), // `GetShowObjectives` 0x15995e0
    ("GetShowObservedDelay", "ShowObservedDelay"), // `GetShowObservedDelay` 0x15995f0
    ("GetShowScoreFeed", "ShowScoreFeed"), // `GetShowScoreFeed` 0x1599600
    ("GetShowServerInScoreboard", "ShowServerInScoreboard"), // `GetShowServerInScoreboard` 0x1599610
    ("GetShowSpawnInfo", "ShowSpawnInfo"), // `GetShowSpawnInfo` 0x1599620
    ("GetShowStatusBar", "ShowStatusBar"), // `GetShowStatusBar` 0x1599630
    ("GetShowTargetInfo", "ShowTargetInfo"), // `GetShowTargetInfo` 0x1599640
    ("GetShowTips", "ShowTips"), // `GetShowTips` 0x1599650
    ("GetThirdPersonDeathCamera", "ThirdPersonDeathCamera"), // `GetThirdPersonDeathCamera` 0x1599cd0
    ("GetTracersStayTime", "DrawTracersStayTime"), // `GetTracersStayTime` 0x1599d00
    ("GetVideoVolume", "VideoVolume"), // `GetVideoVolume` 0x1599d30
    ("GetVoiceVolume", "VoiceVolume"), // `GetVoiceVolume` 0x1599d40
];
/// SETTERS: (UFUNCTION, ini key of the field it stores)
pub const SETTERS: &[(&str, &str)] = &[
    ("SetAmbientOcclusion", "AmbientOcclusion"), // `SetAmbientOcclusion` 0x15a7100
    ("SetAntiAliasing", "AntiAliasing"), // `SetAntiAliasing` 0x15a7130
    ("SetBloom", "Bloom"), // `SetBloom` 0x15a7140
    ("SetCameraDistance", "CameraDistance"), // `SetCameraDistance` 0x15a7150
    ("SetCasualMatchmakingRegion", "CasualMatchmakingRegion"), // `SetCasualMatchmakingRegion` 0x15a7170
    ("SetCharacterCloth", "CharacterCloth"), // `SetCharacterCloth` 0x15a7180
    ("SetCharacterFidelity", "CharacterFidelity"), // `SetCharacterFidelity` 0x15a7190
    ("SetCombatHeadbob", "CombatHeadbob"), // `SetCombatHeadbob` 0x15a71a0
    ("SetCrosshairType", "CrosshairType"), // `SetCrosshairType` 0x15a71c0
    ("SetCrossplayEnabled", "bCrossplayEnabled"), // `SetCrossplayEnabled` 0x15a71d0
    ("SetDrawTracers", "DrawTracers"), // `SetDrawTracers` 0x15a71f0
    ("SetEffectsVolume", "EffectsVolume"), // `SetEffectsVolume` 0x15a7200
    ("SetFieldOfView", "FieldOfView"), // `SetFieldOfView` 0x15a7220
    ("SetForceFeedbackEnabled", "ForceFeedback"), // `SetForceFeedbackEnabled` 0x15a7230
    ("SetFriendlyMarkers", "FriendlyMarkers"), // `SetFriendlyMarkers` 0x15a7240
    ("SetGamma", "Gamma"), // `SetGamma` 0x15a7300
    ("SetGore", "Gore"), // `SetGore` 0x15a7310
    ("SetHeadbob", "Headbob"), // `SetHeadbob` 0x15a7320
    ("SetHideDefaultLoadouts", "HideDefaultLoadouts"), // `SetHideDefaultLoadouts` 0x15a7330
    ("SetHideHUD", "HideHUD"), // `SetHideHUD` 0x15a7340
    ("SetIndirectCapsuleShadows", "IndirectCapsuleShadows"), // `SetIndirectCapsuleShadows` 0x15a7350
    ("SetInstrumentsVolume", "InstrumentsVolume"), // `SetInstrumentsVolume` 0x15a7360
    ("SetLensFlares", "LensFlares"), // `SetLensFlares` 0x15a73c0
    ("SetMasterVolume", "MasterVolume"), // `SetMasterVolume` 0x15a73d0
    ("SetMaxRagdolls", "MaxRagdolls"), // `SetMaxRagdolls` 0x15a7420
    ("SetMotionBlur", "MotionBlur"), // `SetMotionBlur` 0x15a7430
    ("SetMouseSmoothing", "MouseSmoothing"), // `SetMouseSmoothing` 0x15a7440
    ("SetMovementHeadbob", "MovementHeadbob"), // `SetMovementHeadbob` 0x15a7490
    ("SetMusicVolume", "MusicVolume"), // `SetMusicVolume` 0x15a74a0
    ("SetNoTeamColorsOnGear", "NoTeamColorsOnGear"), // `SetNoTeamColorsOnGear` 0x15a74b0
    ("SetNvidiaReflex", "NvidiaReflex"), // `SetNvidiaReflex` 0x15a74c0
    ("SetPlatformSpecific", "PlatformSpecific"), // `SetPlatformSpecific` 0x15a74d0
    ("SetProfanityFilter", "ProfanityFilter"), // `SetProfanityFilter` 0x15a74e0
    ("SetRagdollFidelity", "RagdollFidelity"), // `SetRagdollFidelity` 0x15a74f0
    ("SetRagdollStayTime", "RagdollStayTime"), // `SetRagdollStayTime` 0x15a7500
    ("SetRangedSensitivity", "RangedSensitivity"), // `SetRangedSensitivity` 0x15a7510
    ("SetRankedMatchmakingRegion", "RankedMatchmakingRegion"), // `SetRankedMatchmakingRegion` 0x15a7530
    ("SetScreenPercentage", "ScreenPercentage"), // `SetScreenPercentage` 0x15a7610
    ("SetScreenSpaceReflections", "ScreenSpaceReflections"), // `SetScreenSpaceReflections` 0x15a7620
    ("SetServerBrowserHasPlayers", "bServerBrowserHasPlayers"), // `SetServerBrowserHasPlayers` 0x15a7710
    ("SetServerBrowserIsConsoleServer", "bServerBrowserConsoleServer"), // `SetServerBrowserIsConsoleServer` 0x15a7720
    ("SetServerBrowserIsOfficial", "bServerBrowserIsOfficial"), // `SetServerBrowserIsOfficial` 0x15a7730
    ("SetServerBrowserMaxPing", "ServerBrowserMaxPing"), // `SetServerBrowserMaxPing` 0x15a7740
    ("SetServerBrowserNoPassword", "bServerBrowserNoPassword"), // `SetServerBrowserNoPassword` 0x15a7750
    ("SetServerBrowserNotFull", "bServerBrowserNotFull"), // `SetServerBrowserNotFull` 0x15a7760
    ("SetShowAmmo", "ShowAmmo"), // `SetShowAmmo` 0x15a77d0
    ("SetShowAnnouncements", "ShowAnnouncements"), // `SetShowAnnouncements` 0x15a77e0
    ("SetShowChatBox", "ShowChatBox"), // `SetShowChatBox` 0x15a77f0
    ("SetShowCombatHints", "ShowCombatHints"), // `SetShowCombatHints` 0x15a7800
    ("SetShowEmotesMenu", "ShowEmotesMenu"), // `SetShowEmotesMenu` 0x15a7810
    ("SetShowEquipment", "ShowEquipment"), // `SetShowEquipment` 0x15a7820
    ("SetShowHitMarker", "ShowHitMarker"), // `SetShowHitMarker` 0x15a7830
    ("SetShowKillFeed", "ShowKillFeed"), // `SetShowKillFeed` 0x15a7840
    ("SetShowKilledBy", "ShowKilledBy"), // `SetShowKilledBy` 0x15a7850
    ("SetShowObjectives", "ShowObjectives"), // `SetShowObjectives` 0x15a78e0
    ("SetShowObservedDelay", "ShowObservedDelay"), // `SetShowObservedDelay` 0x15a78f0
    ("SetShowScoreFeed", "ShowScoreFeed"), // `SetShowScoreFeed` 0x15a7900
    ("SetShowServerInScoreboard", "ShowServerInScoreboard"), // `SetShowServerInScoreboard` 0x15a7910
    ("SetShowSpawnInfo", "ShowSpawnInfo"), // `SetShowSpawnInfo` 0x15a7920
    ("SetShowStatusBar", "ShowStatusBar"), // `SetShowStatusBar` 0x15a7930
    ("SetShowTargetInfo", "ShowTargetInfo"), // `SetShowTargetInfo` 0x15a7940
    ("SetShowTips", "ShowTips"), // `SetShowTips` 0x15a7950
    ("SetThirdPersonDeathcamera", "ThirdPersonDeathCamera"), // `SetThirdPersonDeathcamera` 0x15a7c80
    ("SetTracersStayTime", "DrawTracersStayTime"), // `SetTracersStayTime` 0x15a8310
    ("SetVideoVolume", "VideoVolume"), // `SetVideoVolume` 0x15a8320
    ("SetVoiceVolume", "VoiceVolume"), // `SetVoiceVolume` 0x15a8330
];
/// Get*Limits (FVector2D min, max): (UFUNCTION, min, max)
pub const LIMITS: &[(&str, f64, f64)] = &[
    ("GetBloomLimits", 0.0, 1.0), // `GetBloomLimits` 0x9d8030
    ("GetCameraDistanceLimits", -15.0, 15.0), // `GetCameraDistanceLimits` 0x15951c0
    ("GetCombatHeadbobLimits", 0.0, 2.0), // `GetCombatHeadbobLimits` 0x1595300
    ("GetFieldOfViewLimits", 30.0, 101.0), // `GetFieldOfViewLimits` 0x1595df0
    ("GetFrameRateLimits", 30.0, 250.0), // `GetFrameRateLimits` 0x1595e20
    ("GetMotionBlurLimits", 0.0, 0.3), // `GetMotionBlurLimits` 0x1596cc0
    ("GetMouseSmoothingLimits", 0.0, 4.0), // `GetMouseSmoothingLimits` 0x1596d10
    ("GetRangedSensitivityLimits", 0.1, 1.0), // `GetRangedSensitivityLimits` 0x1598b10
    ("GetScreenPercentageLimits", 25.0, 200.0), // `GetScreenPercentageLimits` 0x1598d40
    ("GetTracersStayTimeLimits", 1.0, 10.0), // `GetTracersStayTimeLimits` 0x1599d10
];

/// ICF aliases of the 0..2 limits function `GetCombatHeadbobLimits` 0x1595300 (GetGammaLimits, GetHeadbobLimits,
/// GetMovementHeadbobLimits)
pub fn limits(name: &str) -> Option<(f64, f64)> {
    match name {
        "GetGammaLimits" | "GetHeadbobLimits" | "GetMovementHeadbobLimits" => Some((0.0, 2.0)),
        _ => LIMITS.iter().find(|l| l.0 == name).map(|l| (l.1, l.2)),
    }
}

/// UEngine scalability groups (Get/Set<X>Quality -> sg.<X>Quality, UE 4.26 UGameUserSettings::ScalabilityQuality)
const SCALABILITY: &[(&str, &str)] = &[
    ("ViewDistanceQuality", "sg.ViewDistanceQuality"),
    ("AntiAliasingQuality", "sg.AntiAliasingQuality"),
    ("ShadowQuality", "sg.ShadowQuality"),
    ("PostProcessingQuality", "sg.PostProcessQuality"),
    ("TextureQuality", "sg.TextureQuality"),
    ("VisualEffectQuality", "sg.EffectsQuality"),
    ("FoliageQuality", "sg.FoliageQuality"),
    ("ShadingQuality", "sg.ShadingQuality"),
];

/// UGameUserSettings fields the screens read / write (engine config keys)
const ENGINE_KEYS: &[&str] = &["FullscreenMode", "FrameRateLimit", "bUseVSync", "ResolutionSizeX", "ResolutionSizeY", "AudioQualityLevel", "Language"];

pub fn config_dir() -> PathBuf {
    if let Ok(d) = std::env::var("MH_CONFIG_DIR") {
        return PathBuf::from(d);
    }
    let base = std::env::var("LOCALAPPDATA").unwrap_or_default();
    PathBuf::from(base).join("MordhauRewrite").join("Saved").join("Config").join("WindowsClient")
}

/// ini text -> value (an untyped config string: bool / int / float / name)
fn parse(s: &str) -> V {
    match s {
        "True" | "true" => V::Bool(true),
        "False" | "false" => V::Bool(false),
        _ => {
            if let Ok(i) = s.parse::<i64>() {
                V::Int(i)
            } else if let Ok(f) = s.parse::<f64>() {
                V::Float(f)
            } else {
                V::Name(s.to_string())
            }
        }
    }
}

/// value -> ini text (FProperty::ExportText: floats "%f", bools True / False)
fn export(v: &V) -> String {
    match v {
        V::Bool(b) => if *b { "True" } else { "False" }.into(),
        V::Int(i) => i.to_string(),
        V::Float(f) => format!("{f:.6}"),
        v => v.s(),
    }
}

/// UMordhauGameUserSettings::SetToDefaults 0x15a7c90 into the settings object
pub fn set_to_defaults(vm: &mut Vm, o: Id) {
    for (k, f) in DEFAULTS {
        vm.set(o, k, f());
    }
}

/// LoadSettings: defaults, then the ini section and [ScalabilityGroups]
pub fn load(vm: &mut Vm, o: Id) {
    set_to_defaults(vm, o);
    let p = config_dir().join("GameUserSettings.ini");
    let Ok(t) = std::fs::read_to_string(&p) else { return };
    let mut sec = String::new();
    for l in t.lines() {
        let l = l.trim_end_matches('\r');
        if l.starts_with('[') {
            sec = l.to_string();
            continue;
        }
        let Some((k, v)) = l.split_once('=') else { continue };
        if sec == SECTION || sec == "[ScalabilityGroups]" {
            vm.set(o, k, parse(v));
        }
    }
}

fn known(k: &str) -> bool {
    DEFAULTS.iter().any(|d| d.0 == k) || SCALABILITY.iter().any(|s| s.1 == k) || ENGINE_KEYS.contains(&k)
}

/// SaveSettings: rewrite the known keys of the section / [ScalabilityGroups] in place
pub fn save(vm: &Vm, o: Id) -> std::io::Result<()> {
    let p = config_dir().join("GameUserSettings.ini");
    let t = std::fs::read_to_string(&p).unwrap_or_else(|_| format!("{SECTION}\n"));
    let mut out = String::new();
    let mut sec = String::new();
    let mut written = std::collections::HashSet::new();
    for l in t.lines() {
        let l2 = l.trim_end_matches('\r');
        if l2.starts_with('[') {
            sec = l2.to_string();
        } else if sec == SECTION || sec == "[ScalabilityGroups]" {
            if let Some((k, _)) = l2.split_once('=') {
                if known(k) {
                    let v = vm.prop(o, k);
                    if !matches!(v, V::None) {
                        out.push_str(&format!("{k}={}\n", export(&v)));
                        written.insert(k.to_string());
                        continue;
                    }
                }
            }
        }
        out.push_str(l2);
        out.push('\n');
    }
    // keys the file did not have yet go at the start of the section
    let missing: Vec<&str> = DEFAULTS.iter().map(|d| d.0).filter(|k| !written.contains(*k)).collect();
    if !missing.is_empty() {
        let mut add = String::new();
        for k in missing {
            add.push_str(&format!("{k}={}\n", export(&vm.prop(o, k))));
        }
        match out.find(SECTION) {
            Some(i) => {
                let at = out[i..].find('\n').map(|j| i + j + 1).unwrap_or(out.len());
                out.insert_str(at, &add);
            }
            None => out = format!("{SECTION}\n{add}\n{out}"),
        }
    }
    // the rewrite's own config dir does not exist before the first save (fidelity-audit r7)
    if let Some(d) = std::path::Path::new(&p).parent() {
        let _ = std::fs::create_dir_all(d);
    }
    std::fs::write(&p, out)
}

/// The settings object's natives (the target of GetMordhauGameUserSettings()): Get / Set by the exe's field tables,
/// Get*Limits, scalability groups, engine fields, ApplySettings / SaveSettings / SetToDefaults
pub fn call(vm: &mut Vm, o: Id, name: &str, a: &[V]) -> Option<V> {
    let a0 = a.first().cloned().unwrap_or_default();
    if let Some((_, k)) = GETTERS.iter().find(|g| g.0 == name) {
        return Some(vm.prop(o, k));
    }
    if let Some((_, k)) = SETTERS.iter().find(|g| g.0 == name) {
        vm.set(o, k, a0);
        return Some(V::None);
    }
    if let Some((lo, hi)) = limits(name) {
        return Some(V::st(&[("X", V::Float(lo)), ("Y", V::Float(hi))]));
    }
    let set = |vm: &mut Vm, k: &str, v: V| {
        vm.set(o, k, v);
        Some(V::None)
    };
    match name {
        // `GetMaxRagdollsLimit` 0x1596ca0 (20), `GetRagdollStayTimeLimit` 0x1598af0 (120.0)
        "GetMaxRagdollsLimit" => Some(V::Int(20)),
        "GetRagdollStayTimeLimit" => Some(V::Float(120.0)),
        // `GetQuickSpawn` 0x7bf3a0 returns 0, `GetHideWatermark` 0x809a90 returns 1; `SetQuickSpawn` /
        // `SetHideWatermark` 0x7bf350 store nothing
        "GetQuickSpawn" => Some(V::Int(0)),
        "GetHideWatermark" => Some(V::Int(1)),
        "SetQuickSpawn" | "SetHideWatermark" => Some(V::None),
        // `SetDefaultRangedSensitivity` 0x15a71e0
        "SetDefaultRangedSensitivity" => set(vm, "RangedSensitivity", V::Float(0.35)),
        "SetToDefaults" | "RestoreDefaultSettings" => {
            set_to_defaults(vm, o);
            Some(V::None)
        }
        "LoadSettings" => {
            load(vm, o);
            Some(V::None)
        }
        // UGameUserSettings::ApplySettings = ApplyResolutionSettings + ApplyNonResolutionSettings + SaveSettings
        // (`ApplyNonResolutionSettings` 0x1582870 copies fields into cvars; the runtime reads the fields)
        "ApplySettings" | "SaveSettings" | "ConfirmVideoMode" => {
            if !vm.prop(o, "__readonly").truthy() {
                if let Err(e) = save(vm, o) {
                    eprintln!("mh-ui: settings not saved: {e}");
                }
            }
            vm.actions.push(crate::vm::Action::SettingsApplied { input: false });
            let n = vm.prop(o, "__saved").i() + 1;
            set(vm, "__saved", V::Int(n))
        }
        "ApplyNonResolutionSettings" | "ApplyResolutionSettings" | "ApplyHardwareBenchmarkResults" | "ApplyAudioVolumes" => Some(V::None),
        "GetFullscreenMode" | "GetLastConfirmedFullscreenMode" | "GetPreferredFullscreenMode" => Some(V::Int(vm.prop(o, "FullscreenMode").i())),
        "SetFullscreenMode" => set(vm, "FullscreenMode", V::Int(a0.i())),
        "GetFrameRateLimit" => Some(V::Float(vm.prop(o, "FrameRateLimit").f())),
        "SetFrameRateLimit" => set(vm, "FrameRateLimit", V::Float(a0.f())),
        "IsVSyncEnabled" => Some(V::Bool(vm.prop(o, "bUseVSync").truthy())),
        "SetVSyncEnabled" => set(vm, "bUseVSync", V::Bool(a0.truthy())),
        "GetScreenResolution" | "GetLastConfirmedScreenResolution" | "GetDesktopResolution" => {
            let (mut x, mut y) = (vm.prop(o, "ResolutionSizeX").i(), vm.prop(o, "ResolutionSizeY").i());
            if x <= 0 || y <= 0 {
                // a fresh config has no resolution (UGameUserSettings::SetToDefaults: GetDefaultResolution() = 0x0); the
                // engine then opens its window at the desktop size (UE 4.26 UGameEngine::DetermineGameWindowResolution)
                // and ConfirmVideoMode stores that, so the game reports the window's size: here the viewport the host
                // passes to UiRuntime::update (UNCONFIRMED in the exe: engine functions not disassembled)
                let s = vm.world.get("viewport").map(|&v| vm.prop(v, "Size")).unwrap_or(V::None);
                (x, y) = (s.field("X").f().round() as i64, s.field("Y").f().round() as i64);
            }
            Some(V::st(&[("X", V::Int(x)), ("Y", V::Int(y))]))
        }
        "SetScreenResolution" => {
            vm.set(o, "ResolutionSizeX", V::Int(a0.field("X").i()));
            set(vm, "ResolutionSizeY", V::Int(a0.field("Y").i()))
        }
        "GetAudioQualityLevel" => Some(V::Int(vm.prop(o, "AudioQualityLevel").i())),
        "SetAudioQualityLevel" => set(vm, "AudioQualityLevel", V::Int(a0.i())),
        "GetLanguage" => Some(V::Str(vm.prop(o, "Language").s())),
        "SetLanguage" => set(vm, "Language", V::Str(a0.s())),
        // ShouldShow<X> (e.g. `ShouldShowStatusBar` 0x15a88d0, `ShouldShowKillFeed` 0x15a86b0): CVarHideHUD == 0 and
        // CVarShow<X> != 0; `ShouldShowHUD` 0x15a8630: HideHUD == 0; `ShouldDrawTracers` 0x15a8390 DrawTracers != 0;
        // `ShouldShowBlood` 0x15a8510 Gore != 0. The cvars hold the fields after ApplyNonResolutionSettings
        // 0x1582870 (applied on load), so the fields are read. `ShouldQuickSpawn` / `ShouldShowWatermark` 0x7bf520: false
        // `GetActualCrosshairType` 0x1594aa0: CVarCrosshairType (= the CrosshairType field once applied)
        "GetActualCrosshairType" => Some(V::Int(vm.prop(o, "CrosshairType").i())),
        "ShouldShowHUD" => Some(V::Bool(vm.prop(o, "HideHUD").i() == 0)),
        "ShouldDrawTracers" => Some(V::Bool(vm.prop(o, "DrawTracers").i() != 0)),
        "ShouldShowBlood" => Some(V::Bool(vm.prop(o, "Gore").i() != 0)),
        "ShouldQuickSpawn" | "ShouldShowWatermark" => Some(V::Bool(false)),
        "ShouldShowObservedDelay" | "ShouldShowServerInScoreboard" | "ShouldShowMatchmakingDebug" => Some(V::Bool(vm.prop(o, &name["Should".len()..]).i() != 0)),
        _ if name.starts_with("ShouldShow") => {
            let k = &name["Should".len()..];
            let v = vm.prop(o, k);
            Some(V::Bool(vm.prop(o, "HideHUD").i() == 0 && (matches!(v, V::None) || v.i() != 0)))
        }
        "GetAvailableLanguages" => Some(V::Array(available_languages(vm).into_iter().map(V::Str).collect())),
        _ => {
            for (n, k) in SCALABILITY {
                if name.strip_prefix("Get") == Some(n) {
                    // a group the ini does not have keeps its cvar default: 3 (Epic, the top level; UE 4.26
                    // Scalability.cpp registers every sg.<Group>Quality with default 3, Scalability::LoadState keeps
                    // it when [ScalabilityGroups] lacks the key). A fresh install therefore starts at the top
                    // ("Ultra" in the menu), as the real game's fresh GameUserSettings.ini shows (all sg.* = 3, no
                    // hardware benchmark: LastGPUBenchmarkResult=-1)
                    return Some(V::Int(match vm.prop(o, k) {
                        V::None => 3,
                        v => v.i(),
                    }));
                }
                if name.strip_prefix("Set") == Some(n) {
                    return set(vm, k, V::Int(a0.i()));
                }
            }
            None
        }
    }
}

/// AvailableLanguages, built by the ctor `UMordhauGameUserSettings::UMordhauGameUserSettings` 0x157f020 from
/// FTextLocalizationManager::GetLocalizedCultureNames (the paks' Mordhau/Content/Localization/Game/<culture>/ folders)
/// through FInternationalization::GetAvailableCultures, one display name per culture. The names are ICU's English
/// display names (UNCONFIRMED: which FCulture name accessor the ctor uses; the user's ini stores "English")
pub fn available_languages(vm: &Vm) -> Vec<String> {
    let Some(src) = vm.src.as_ref() else { return vec!["English".into()] };
    let mut cultures: Vec<String> = src
        .vfs
        .list()
        .filter_map(|p| p.strip_prefix("Mordhau/Content/Localization/Game/").and_then(|r| r.strip_suffix("/Game.locres")).map(str::to_string))
        .collect();
    cultures.sort();
    cultures
        .iter()
        .map(|c| {
            match c.as_str() {
                "de" => "German",
                "en" => "English",
                "es" => "Spanish",
                "es-419" => "Spanish (Latin America)",
                "fr" => "French",
                "fr-CA" => "French (Canada)",
                "it" => "Italian",
                "ja" => "Japanese",
                "ko" => "Korean",
                "pt" => "Portuguese",
                "pt-BR" => "Portuguese (Brazil)",
                "ru" => "Russian",
                "zh-Hans" => "Chinese (Simplified)",
                "zh-Hant" => "Chinese (Traditional)",
                other => other,
            }
            .to_string()
        })
        .collect()
}

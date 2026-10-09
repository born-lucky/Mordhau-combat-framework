//! usersettings.rs - the player's UMordhauGameUserSettings (fidelity-audit r2): the saved
//! GameUserSettings.ini in the rewrite's own config dir (mh_ui::settings::config_dir; never the Steam install's) [/Script/Mordhau.MordhauGameUserSettings]
//! over SetToDefaults rva=0x15a7c90 (decomp UMordhauGameUserSettings.cpp 764-...). ApplyNonResolutionSettings
//! rva=0x1582870 copies them into the m.* cvars the game reads (m.FieldOfView decomp ~402, m.Gore decomp 60).

use std::collections::HashMap;

#[derive(bevy::prelude::Resource, Clone, Debug, serde::Serialize)]
pub struct UserSettings {
    /// m.FieldOfView (SetToDefaults 78.0, decomp 888)
    pub field_of_view: f32,
    /// m.Gore (SetToDefaults 2, decomp 828); UMordhauGameUserSettings::ShouldShowBlood rva=0x15a8510 = m.Gore != 0
    pub gore: i32,
    /// m.Headbob (UMordhauCameraComponent::DoCameraShakeIfViewTarget rva=0x14ba4f0 cvar "m.Headbob" .rdata 0x14432d208)
    pub headbob: f32,
    /// DrawTracers / DrawTracersStayTime (GameUserSettings.ini; mh-ui settings.rs defaults 0 / 2.0 as the game's
    /// GetDrawTracers rva=0x1595bb0 / GetTracersStayTime rva=0x1599d00 read them): UMordhauGameUserSettings::
    /// ShouldDrawTracers rva=0x15a8390 = the CVarDrawTracers it is applied to != 0; AMordhauWeapon's trace loop (decomp
    /// AMordhauWeapon.cpp 216-252) then draws every trace step's blade segment: on the owning client through
    /// AMordhauPlayerController::ClientDrawTracer (DrawDebugLine green, the stay time, thickness 0.5), on the authority
    /// yellow (blue for the cosmetic second trace)
    pub draw_tracers: i32,
    pub draw_tracers_stay_time: f32,
    pub source: String,
}

impl Default for UserSettings {
    fn default() -> Self {
        // UMordhauGameUserSettings::SetToDefaults rva=0x15a7c90: FieldOfView 78, Gore 2, Headbob 1 (decomp 828, 888, 834);
        // a fresh install (no saved GameUserSettings.ini, Version 0) gets them through UGameUserSettings::ValidateSettings
        // (UE 4.26 engine source; UNCONFIRMED in the decomp)
        UserSettings { field_of_view: 78.0, gore: 2, headbob: 1.0, draw_tracers: 0, draw_tracers_stay_time: 2.0, source: "SetToDefaults".into() }
    }
}

/// GetFieldOfViewLimits rva=0x1595df0: (30, 101)
pub const FOV_LIMITS: (f32, f32) = (30.0, 101.0);

pub fn parse(text: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let mut in_sec = false;
    for l in text.lines().map(str::trim) {
        if l.starts_with('[') {
            in_sec = l == "[/Script/Mordhau.MordhauGameUserSettings]";
        } else if in_sec {
            if let Some((k, v)) = l.split_once('=') {
                out.entry(k.to_string()).or_insert_with(|| v.to_string());
            }
        }
    }
    out
}

impl UserSettings {
    pub fn from_map(m: &HashMap<String, String>, source: &str) -> UserSettings {
        let d = UserSettings::default();
        let f = |k: &str, x: f32| m.get(k).and_then(|v| v.trim().parse().ok()).unwrap_or(x);
        UserSettings {
            field_of_view: f("FieldOfView", d.field_of_view),
            gore: m.get("Gore").and_then(|v| v.trim().parse().ok()).unwrap_or(d.gore),
            headbob: f("Headbob", d.headbob),
            draw_tracers: m.get("DrawTracers").and_then(|v| v.trim().parse().ok()).unwrap_or(d.draw_tracers),
            draw_tracers_stay_time: f("DrawTracersStayTime", d.draw_tracers_stay_time),
            source: source.into(),
        }
    }
    pub fn load() -> UserSettings {
        // the rewrite's own config dir (mh_ui::settings::config_dir: $MH_CONFIG_DIR or %LOCALAPPDATA%\MordhauRewrite\...),
        // never the Steam install's; MORDHAU_GUS_INI names a file explicitly (tests)
        let p = std::env::var("MORDHAU_GUS_INI")
            .ok()
            .map(std::path::PathBuf::from)
            .or_else(|| Some(mh_ui::settings::config_dir().join("GameUserSettings.ini")));
        match p.as_ref().and_then(|p| std::fs::read_to_string(p).ok()) {
            Some(t) => UserSettings::from_map(&parse(&t), &p.unwrap().display().to_string()),
            None => UserSettings::default(),
        }
    }
    pub fn show_blood(&self) -> bool {
        self.gore != 0
    }
    /// UMordhauCameraComponent::ComputeCameraPOV rva=0x14b5520 (decomp 196-220): clamp(m.FieldOfView, limits) +
    /// CurrentSpeedFOVOffset + CurrentMotionFOVOffset, clamped to the limits; third person capped at MaxThirdPersonFOV
    pub fn pov_fov(&self, speed_offset: f32, motion_offset: f32, third_person: bool, max_3p: f32) -> f32 {
        let (lo, hi) = FOV_LIMITS;
        let base = self.field_of_view.clamp(lo, hi);
        let f = (base + speed_offset + motion_offset).clamp(lo, hi);
        if third_person && max_3p <= f { max_3p } else { f }
    }
}

/// FMath::FInterpTo
pub fn finterp_to(cur: f32, target: f32, dt: f32, speed: f32) -> f32 {
    if speed <= 0.0 {
        return target;
    }
    let d = target - cur;
    if d * d < 1e-8 {
        return target;
    }
    cur + d * (dt * speed).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_fov_and_gore() {
        let m = parse("[/Script/Mordhau.MordhauGameUserSettings]\nGore=0\nFieldOfView=93.000000\n[/Script/Engine.GameUserSettings]\nGore=5\n");
        let u = UserSettings::from_map(&m, "t");
        assert_eq!(u.field_of_view, 93.0);
        assert!(!u.show_blood());
        // 93 + sprint 5 = 98; 3P capped at MaxThirdPersonFOV 78
        assert_eq!(u.pov_fov(5.0, 0.0, false, 78.0), 98.0);
        assert_eq!(u.pov_fov(5.0, 0.0, true, 78.0), 78.0);
        assert_eq!(UserSettings { field_of_view: 120.0, ..u.clone() }.pov_fov(5.0, 0.0, false, 200.0), 101.0);
        assert_eq!(UserSettings::default().field_of_view, 78.0);
    }
}

//! The game's shipped config files read from the paks (port of `records/ue_config.gd` `UeConfig.path/value/float_value`
//! and `ue_light.gd` `ini_float`). The project's Default*.ini live under Mordhau/Config, the engine's Base*.ini under
//! Engine/Config (UE's config hierarchy, FConfigCacheIni; paths as in extract/manifest.tsv).

use mh_pak::Vfs;

/// Mounted path of a config file ("DefaultEngine.ini" -> "Mordhau/Config/DefaultEngine.ini")
pub fn path(file: &str) -> String {
    format!("{}{file}", if file.starts_with("Base") { "Engine/Config/" } else { "Mordhau/Config/" })
}

/// Text of a config file from the paks; None when missing
pub fn text(vfs: &Vfs, file: &str) -> Option<String> {
    vfs.read(&path(file), false).map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// Value of `key` in `[section]` ("" when the file, section or key is missing). Plain key=value lines; the first match
/// wins (UeConfig.value)
pub fn value(vfs: &Vfs, file: &str, section: &str, key: &str) -> String {
    let Some(t) = text(vfs, file) else { return String::new() };
    let head = format!("[{section}]");
    let pre = format!("{key}=");
    let mut in_sec = false;
    for raw in t.lines() {
        let l = raw.trim();
        if l.starts_with('[') {
            in_sec = l == head;
        } else if in_sec && l.starts_with(&pre) {
            return l[pre.len()..].to_string();
        }
    }
    String::new()
}

/// A float value; "44.f" (a C++ float literal in BaseEngine.ini) reads as 44. None when missing or not a number
pub fn float_value(vfs: &Vfs, file: &str, section: &str, key: &str) -> Option<f64> {
    parse_f(&value(vfs, file, section, key))
}

fn parse_f(s: &str) -> Option<f64> {
    let s = s.trim();
    let s = s.strip_suffix('f').unwrap_or(s);
    s.parse().ok()
}

/// A console variable anywhere in the file (any section), last assignment wins; `dflt` if absent (UeLight.ini_float)
pub fn ini_float(vfs: &Vfs, file: &str, key: &str, dflt: f64) -> f64 {
    let Some(t) = text(vfs, file) else { return dflt };
    let pre = format!("{key}=");
    let mut v = dflt;
    for line in t.split('\n') {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix(&pre) {
            // GDScript String.to_float: the leading number, 0 when there is none
            v = leading_float(rest);
        }
    }
    v
}

fn leading_float(s: &str) -> f64 {
    let end = s
        .char_indices()
        .take_while(|&(i, c)| c.is_ascii_digit() || c == '.' || ((c == '-' || c == '+') && i == 0) || c == 'e' || c == 'E')
        .map(|(i, c)| i + c.len_utf8())
        .last()
        .unwrap_or(0);
    let mut e = end;
    while e > 0 {
        if let Ok(v) = s[..e].parse() {
            return v;
        }
        e -= 1;
    }
    0.0
}

/// Recast agent of DefaultEngine.ini [/Script/NavigationSystem.RecastNavMesh] (ModeData.nav_agent); AgentMaxSlope
/// comes from BaseEngine.ini
#[derive(Clone, Debug, PartialEq)]
pub struct NavAgent {
    pub radius: f64,
    pub height: f64,
    pub max_step: f64,
    pub cell_size: f64,
    pub cell_height: f64,
    pub max_slope: f64,
}

pub fn nav_agent(vfs: &Vfs) -> NavAgent {
    const S: &str = "/Script/NavigationSystem.RecastNavMesh";
    let f = |k: &str| float_value(vfs, "DefaultEngine.ini", S, k).unwrap_or(f64::NAN);
    NavAgent {
        radius: f("AgentRadius"),
        height: f("AgentHeight"),
        max_step: f("AgentMaxStepHeight"),
        cell_size: f("CellSize"),
        cell_height: f("CellHeight"),
        max_slope: float_value(vfs, "BaseEngine.ini", S, "AgentMaxSlope").unwrap_or(f64::NAN),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn leading_float_like_gdscript() {
        assert_eq!(super::leading_float("0.5 ; comment"), 0.5);
        assert_eq!(super::leading_float("-2"), -2.0);
        assert_eq!(super::leading_float("x"), 0.0);
        assert_eq!(super::parse_f("44.f"), Some(44.0));
    }
}

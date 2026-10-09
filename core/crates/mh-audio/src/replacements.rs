//! Framework sample bank. SoundCue scheduling/mixing remain native; only PCM leaves change.
use crate::mixer::{decode_ogg, Pcm};
use serde_json::Value;
use std::{collections::HashMap, path::{Component, Path, PathBuf}, sync::Arc};

pub struct Bank {
    root: PathBuf,
    roles: HashMap<String, Vec<String>>,
    decoded: HashMap<String, Pcm>,
}

impl Bank {
    pub fn open(root: &Path) -> Result<Self, String> {
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        let manifest: Value = serde_json::from_slice(&std::fs::read(root.join("bank.json")).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        if manifest["version"].as_u64() != Some(1) { return Err("Unsupported audio bank version".into()); }
        let mut bank = Self { root, roles: HashMap::new(), decoded: HashMap::new() };
        for (role, files) in manifest["roles"].as_object().ok_or("Missing roles")? {
            let mut names = vec![];
            for name in files.as_array().ok_or("Role must contain an array")? {
                let name = name.as_str().ok_or("Sample filename must be a string")?;
                let path = Path::new(name);
                if path.components().any(|c| !matches!(c, Component::Normal(_))) { return Err(format!("Unsafe sample path: {name}")); }
                let full = bank.root.join(path).canonicalize().map_err(|e| format!("{name}: {e}"))?;
                if !full.starts_with(&bank.root) { return Err(format!("Sample escapes bank: {name}")); }
                if !bank.decoded.contains_key(name) {
                    let bytes = std::fs::read(full).map_err(|e| e.to_string())?;
                    let mut pcm = if bytes.starts_with(b"OggS") { decode_ogg(&bytes)? } else { decode_wav(&bytes)? };
                    if pcm.channels == 0 || pcm.rate == 0 || pcm.samples.is_empty() { return Err(format!("Empty/invalid sample: {name}")); }
                    // Spatial one-shots use mono so their stereo recording does not widen the native emitter.
                    let mut mono: Vec<i16> = pcm.samples.chunks_exact(pcm.channels as usize)
                        .map(|frame| (frame.iter().map(|&x| x as i32).sum::<i32>() / frame.len() as i32) as i16).collect();
                    let peak = mono.iter().map(|&v| (v as i32).abs()).max().unwrap_or(0);
                    if peak == 0 { return Err(format!("Silent sample: {name}")); }
                    // Remove only surrounding recording silence; never time-stretch an impact or change event time.
                    let threshold = (peak / 1000).max(16);
                    let start = mono.iter().position(|&v| (v as i32).abs() > threshold).unwrap_or(0);
                    let end = mono.iter().rposition(|&v| (v as i32).abs() > threshold).unwrap_or(mono.len()-1) + 1;
                    let pad = pcm.rate as usize / 200;
                    mono = mono[start.saturating_sub(pad)..(end + pad).min(mono.len())].to_vec();
                    // Consistent peak headroom; cue/node volume and envelopes still determine playback gain.
                    let gain = 0.65 * 32767.0 / peak as f32;
                    for x in &mut mono { *x = (*x as f32 * gain).round().clamp(-32768.0, 32767.0) as i16; }
                    pcm.channels = 1;
                    pcm.samples = Arc::new(mono);
                    bank.decoded.insert(name.to_owned(), pcm);
                }
                names.push(name.to_owned());
            }
            if names.is_empty() { return Err(format!("Empty sample role: {role}")); }
            bank.roles.insert(role.clone(), names);
        }
        Ok(bank)
    }

    pub fn sample(&self, cue: &str, wave: &str) -> Option<(&str, &str, Pcm)> {
        let role = role(cue, wave)?;
        let files = self.roles.get(role)?;
        // Stable variation per native wave without consuming the native SoundCue RNG stream.
        let hash = wave.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
        let name = &files[hash as usize % files.len()];
        Some((role, name, self.decoded[name].clone()))
    }

    pub fn sample_count(&self) -> usize { self.decoded.len() }

    pub fn root(&self) -> &Path { &self.root }
}

/// Classify semantic cue first (many native wave names do not describe their role).
pub fn role(cue: &str, wave: &str) -> Option<&'static str> {
    let c = cue.to_ascii_lowercase();
    let w = wave.to_ascii_lowercase();
    let s = format!("{c} {w}");
    if s.contains("/voices/") {
        if c.contains("death") || w.contains("death") { return Some("voice_death"); }
        if c.contains("hurt") || c.contains("pain") || w.contains("hurt") { return Some("voice_pain"); }
        if c.contains("attack") || c.contains("jump") || c.contains("parry") || w.contains("attack") { return Some("voice_effort"); }
        // Spoken commands/breathing need their own recordings, not an unrelated grunt.
        return None;
    }
    if s.contains("/footsteps/") {
        return Some(if s.contains("grass") || s.contains("dirt") || s.contains("mud") || s.contains("sand") { "step_grass" }
            else if s.contains("wood") { "step_wood" } else if s.contains("snow") { "step_snow" } else { "step_stone" });
    }
    if s.contains("/armor/") || s.contains("/foley/") { return Some("foley"); }
    if s.contains("/gore/") { return Some("flesh"); }
    if s.contains("/ui/") && !s.contains("heartbeat") { return Some("ui"); }
    if s.contains("/weapons/") {
        if s.contains("woosh") || s.contains("flourish") { return Some("swing"); }
        if s.contains("/block") || s.contains("parry") || s.contains("clash") || s.contains("chamber") {
            return Some(if s.contains("wood") { "wood" } else { "parry" });
        }
        if s.contains("environment") || s.contains("environnent") || s.contains("/impacts/") {
            return Some(if w.contains("wood") { "wood" } else if w.contains("cloth") || w.contains("dirt") { "soft" }
                else if w.contains("stone") || w.contains("boulder") { "stone" } else { "metal" });
        }
        if s.contains("/hits/") { return Some(if w.contains("armor") { "metal" } else { "flesh" }); }
        if s.contains("equip") || s.contains("sheat") { return Some("equip"); }
    }
    None
}

/// Installer bank WAV contract: uncompressed RIFF little-endian PCM16, mono/stereo.
fn decode_wav(bytes: &[u8]) -> Result<Pcm, String> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" { return Err("Expected Ogg Vorbis or RIFF WAV".into()); }
    let mut fmt = None;
    let mut data = None;
    let mut offset = 12usize;
    while offset + 8 <= bytes.len() {
        let id = &bytes[offset..offset+4];
        let len = u32::from_le_bytes(bytes[offset+4..offset+8].try_into().unwrap()) as usize;
        let begin = offset + 8;
        let end = begin.checked_add(len).ok_or("WAV chunk overflow")?;
        let chunk = bytes.get(begin..end).ok_or("Truncated WAV chunk")?;
        if id == b"fmt " { fmt = Some(chunk); }
        if id == b"data" { data = Some(chunk); }
        offset = end.checked_add(len % 2).ok_or("WAV offset overflow")?;
    }
    let f = fmt.filter(|f| f.len() >= 16).ok_or("Missing WAV format")?;
    let channels = u16::from_le_bytes(f[2..4].try_into().unwrap());
    let rate = u32::from_le_bytes(f[4..8].try_into().unwrap());
    if u16::from_le_bytes(f[..2].try_into().unwrap()) != 1 || u16::from_le_bytes(f[14..16].try_into().unwrap()) != 16
        || !(1..=2).contains(&channels) || !(8000..=192000).contains(&rate)
        || u16::from_le_bytes(f[12..14].try_into().unwrap()) != channels * 2 { return Err("Bank requires PCM16 mono/stereo WAV".into()); }
    let data = data.ok_or("Missing WAV samples")?;
    if data.is_empty() || data.len() % (channels as usize * 2) != 0 { return Err("Invalid WAV frame count".into()); }
    Ok(Pcm { rate, channels, samples: Arc::new(data.chunks_exact(2).map(|v| i16::from_le_bytes([v[0], v[1]])).collect()) })
}

#[cfg(test)] mod tests {
    use super::*;
    #[test] fn role_keeps_spoken_commands_and_separates_materials() {
        assert_eq!(role("Audio/Cues/Voices/Englishman/SC_EnglishmanAttack", "Attack_Yell_28"), Some("voice_effort"));
        assert_eq!(role("Audio/Cues/Voices/Englishman/SC_EnglishmanDeath", "Deathscream_6"), Some("voice_death"));
        assert_eq!(role("Audio/Cues/Voices/Englishman/SC_EnglishmanHello", "Hello1"), None);
        assert_eq!(role("Audio/Cues/Weapons/Hits/SC_Hit_BladedMassive", "Audio/Raw/Weapons/Hits/BladedLarge/Hit_BladedLarge_1"), Some("flesh"));
        assert_eq!(role("Audio/Cues/Weapons/Environnent/SC_MeleeEnvironmentHit", "SW_MediumArmorMeleeHit14"), Some("metal"));
        assert_eq!(role("Audio/Cues/Weapons/Block/SC_Block", "Audio/Raw/Weapons/Block/Wood/Hit1"), Some("wood"));
    }
    #[test] fn reject_truncated_wav() {
        assert!(decode_wav(b"RIFF\x00\x00\x00\x00WAVEdata\xff\xff\xff\xff").is_err());
        assert!(decode_wav(b"OggS").is_err());
    }
}

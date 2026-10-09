//! Equipment colours, pattern and emblem: what `AMordhauEquipment::UpdateMaterial` 0x1572ed0 sets on every material
//! of an equipment mesh (decomp AMordhauEquipment.cpp 2367-2690).
//!
//! - ColorA / ColorB / ColorC = UMordhauSingleton::GetTableColor(Skins[Skin].ColorTables[k], Colors[k]) (the skin's
//!   per-slot table ids at +0x30): BP_MordhauSingleton ColorTables[table].Entries[index] -> the colour class's Color;
//! - ColorMap = Skins[Skin].Patterns[Pattern].Texture (only when Pattern < the skin's pattern count);
//! - EmblemTexture = Emblems[Emblem] class's Texture, EmblemColorA / B = GetEmblemColor(EmblemColors[k])
//!   (EmblemColorTable.Entries[index].Color) - only set when the emblem has a texture;
//! - team paint (AMordhauGameState::ShouldPaintGearWithTeamColors, bForceTeamColor1 / 2) replaces the emblem colours
//!   (and colour 1 / 2 when forced) with the team colours: not applied here (offline / duel: no team paint;
//!   UNCONFIRMED for team modes).
//! Profile data: DefaultProfiles[i].GearCustomization.Equipment[k] (Id, Colors[3], Skin, Pattern) and
//! AppearanceCustomization (Emblem, EmblemColors[2]); a missing table entry keeps the master default.

use crate::material::{MaterialDesc, TexRef, Uniform};
use mh_pak::Reader;
use serde_json::Value;

pub const SINGLETON: &str = "Mordhau/Content/Mordhau/Blueprints/BP_MordhauSingleton";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EquipmentPaint {
    /// ColorA, ColorB, ColorC (None = that slot's table entry is missing)
    pub colors: [Option<[f32; 3]>; 3],
    pub color_map: Option<TexRef>,
    /// (EmblemTexture, EmblemColorA, EmblemColorB)
    pub emblem: Option<(TexRef, [f32; 3], [f32; 3])>,
}

fn class_color(rd: &Reader, item: &Value) -> Option<[f32; 3]> {
    let p = crate::material::strip_index(item.get("ObjectPath")?.as_str()?).to_string();
    let c = rd.defaults(&p).get("Color").cloned()?;
    let g = |k: &str| c.get(k).and_then(Value::as_f64).unwrap_or(1.0) as f32;
    Some([g("R"), g("G"), g("B")])
}

/// GetEmblemColor: EmblemColorTable.Entries[i].Color
pub fn emblem_color(rd: &Reader, i: i64) -> Option<[f32; 3]> {
    let s = rd.defaults(SINGLETON);
    class_color(rd, s.get("EmblemColorTable")?.get("Entries")?.as_array()?.get(usize::try_from(i).ok()?)?)
}

/// Emblems[i] class's Texture
pub fn emblem_texture(rd: &Reader, i: i64) -> Option<TexRef> {
    let s = rd.defaults(SINGLETON);
    let e = s.get("Emblems")?.as_array()?.get(usize::try_from(i).ok()?)?;
    let p = crate::material::strip_index(e.get("ObjectPath")?.as_str()?).to_string();
    let t = rd.defaults(&p).get("Texture").map(crate::material::ue_pkg_path)?;
    (!t.is_empty()).then(|| TexRef(t))
}

/// UpdateMaterial's inputs for one equipment Blueprint
pub fn paint(rd: &Reader, equipment_bp: &str, skin: i64, pattern: i64, colors: [i64; 3], emblem: i64, emblem_colors: [i64; 2]) -> EquipmentPaint {
    let d = rd.defaults(equipment_bp);
    let s = rd.defaults(SINGLETON);
    let mut out = EquipmentPaint::default();
    let sk = d.get("Skins").and_then(Value::as_array).and_then(|a| a.get(usize::try_from(skin).unwrap_or(0)));
    let tables: Vec<i64> = sk.and_then(|k| k.get("ColorTables")).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default();
    let ct = s.get("ColorTables").and_then(Value::as_array);
    for k in 0..3 {
        out.colors[k] = (|| {
            let t = *tables.get(k)?;
            let e = ct?.get(usize::try_from(t).ok()?)?.get("Entries")?.as_array()?.get(usize::try_from(colors[k]).ok()?)?;
            class_color(rd, e)
        })();
    }
    out.color_map = sk
        .and_then(|k| k.get("Patterns"))
        .and_then(Value::as_array)
        .and_then(|a| a.get(usize::try_from(pattern).unwrap_or(0)))
        .and_then(|p| p.get("Texture"))
        .map(crate::material::ue_pkg_path)
        .filter(|t| !t.is_empty())
        .map(TexRef);
    if let Some(t) = emblem_texture(rd, emblem) {
        let a = emblem_color(rd, emblem_colors[0]).unwrap_or([1.0; 3]);
        let b = emblem_color(rd, emblem_colors[1]).unwrap_or([0.0, 0.0, 1.0]);
        out.emblem = Some((t, a, b));
    }
    out
}

/// the MID parameters on a resolved material (ue_tint uniforms of its mode: color_a/b/c, the ColorMap as color_mask,
/// the emblem slots where the mode samples them)
pub fn apply(m: &mut MaterialDesc, p: &EquipmentPaint) {
    for (k, n) in ["color_a", "color_b", "color_c"].iter().enumerate() {
        if let Some(c) = p.colors[k] {
            m.uniforms.insert(n.to_string(), Uniform::V3(c));
        }
    }
    if let Some(t) = &p.color_map {
        m.uniforms.insert("color_mask".into(), Uniform::Tex(t.clone()));
    }
    if let Some((t, a, b)) = &p.emblem {
        if m.uniforms.contains_key("emblem_texture") || m.uniforms.contains_key("emblem_color_a") {
            m.uniforms.insert("emblem_texture".into(), Uniform::Tex(t.clone()));
            m.uniforms.insert("emblem_color_a".into(), Uniform::V3(*a));
            m.uniforms.insert("emblem_color_b".into(), Uniform::V3(*b));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// the Protector profile's kite shield (Equipment Id 9, Colors [9, 1, 0], Skin 1 = Crusader's Shield, Pattern 0):
    /// three table colours and the cross pattern
    #[test]
    fn protector_kite_shield() {
        let rd = Reader::new(std::sync::Arc::new(mh_pak::Vfs::mount_default().expect("paks")));
        let p = paint(&rd, "Mordhau/Content/Mordhau/Blueprints/Equipment/Shields/BP_KiteShield", 1, 0, [9, 1, 0], 0, [44, 9]);
        assert!(p.colors.iter().all(Option::is_some), "{p:?}");
        assert!(p.color_map.as_ref().is_some_and(|t| t.0.ends_with("ShieldPaintKite_IdCross")), "{p:?}");
        assert!(p.emblem.as_ref().is_some_and(|e| e.0 .0.ends_with("T_Emblem_Blank")), "{p:?}");
        assert_ne!(p.colors[0], p.colors[1]);
    }
}

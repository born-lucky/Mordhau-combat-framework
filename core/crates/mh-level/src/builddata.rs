//! A level's baked lighting: the MapBuildDataRegistry export of <Level>_BuiltData (Level.Properties.MapBuildData),
//! natively serialized. Port of what `ue_lightmap.gd` reads from extract/json (`registry`, `for_component`), decoded
//! here from the bytes. Layout: CUE4Parse UE4/Assets/Exports/BuildData/UMapBuildDataRegistry.cs at the pinned commit:
//!   after UObject's properties: FStripDataFlags (:27); unless audio-visual data is stripped:
//!   TMap<FGuid, FMeshMapBuildData> MeshBuildData (:31), TMap<FGuid, FPrecomputedLightVolumeData> (:32),
//!   TMap<FGuid, FPrecomputedVolumetricLightmapData> (:34-37, FRenderingObjectVersion VolumetricLightmaps),
//!   TMap<FGuid, FLightComponentMapBuildData> LightBuildData (:39), ... (reflection captures, sky atmosphere: not read).
//!   FMeshMapBuildData (:406-452): uint32 ELightMapType + FLightMap2D (:513-603) | none, uint32 EShadowMapType +
//!   FShadowMap2D (:618-643) | none, TArray<FGuid> IrrelevantLights, bulk TArray<FPerInstanceLightmapData {FVector2D
//!   LightmapUVBias, ShadowmapUVBias}>.
//!   FLightMap2D: TArray<FGuid> LightGuids, FPackageIndex Textures[2], SkyOcclusionTexture, AOMaterialMaskTexture,
//!   FVector4 ScaleVectors[i] / AddVectors[i] interleaved for 4 coefficients, FVector2D CoordinateScale, CoordinateBias,
//!   bool bShadowChannelValid[4], FVector4 InvUniformPenumbraSize (LightmapHasShadowmapData), then the virtual
//!   texture index(es) (VirtualTexturedLightmaps V1/V2: one, V3: two FPackageIndex).
//!   FShadowMap2D: TArray<FGuid> LightGuids, FPackageIndex Texture, FVector2D CoordinateScale, CoordinateBias, bool
//!   bChannelValid[4], FVector4 InvUniformPenumbraSize.
//!   FLightComponentMapBuildData (:190-209): int32 ShadowMapChannel, FStaticShadowDepthMapData {FMatrix WorldToLight,
//!   int32 ShadowMapSizeX/Y, TArray<FFloat16> DepthSamples}.
//! The number of virtual-texture indices in FLightMap2D depends on FRenderingObjectVersion, which an unversioned cook
//! does not record: `read` tries two, then one, and keeps the layout whose every entry parses and ends exactly where the
//! next map starts. Mordhau 702625635 is ONE index (VirtualTexturedLightmapsV2): all 876 Arena_BuiltData entries and 57
//! LightBuildData channels then equal extract/json (tests/r3.rs lightmaps_arena); two indices fail to parse.

use crate::level::Pkgs;
use crate::native::{after_props, guid_digits};
use mh_pak::Cursor;
use std::collections::HashMap;

/// A texture reference of the build data: package + export index (decode with mh-assets `texture::info(pkg, name)`)
#[derive(Clone, Debug, PartialEq)]
pub struct TexRef {
    pub pkg: String,
    pub export: usize,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LightMap2D {
    pub light_guids: Vec<String>,
    /// HQ lightmap (top half = coefficient 0, bottom = 1) and LQ textures (LightMapTexture2D)
    pub textures: [Option<TexRef>; 2],
    pub sky_occlusion: Option<TexRef>,
    pub ao_material_mask: Option<TexRef>,
    pub scale_vectors: [[f32; 4]; 4],
    pub add_vectors: [[f32; 4]; 4],
    pub coordinate_scale: [f32; 2],
    pub coordinate_bias: [f32; 2],
    pub shadow_channel_valid: [bool; 4],
    pub inv_uniform_penumbra_size: [f32; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub struct ShadowMap2D {
    pub light_guids: Vec<String>,
    pub texture: Option<TexRef>,
    pub coordinate_scale: [f32; 2],
    pub coordinate_bias: [f32; 2],
    pub channel_valid: [bool; 4],
    pub inv_uniform_penumbra_size: [f32; 4],
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct MeshBuildData {
    pub light_map: Option<LightMap2D>,
    pub shadow_map: Option<ShadowMap2D>,
    pub irrelevant_lights: Vec<String>,
    /// per-instance (LightmapUVBias, ShadowmapUVBias) for instanced components
    pub per_instance: Vec<([f32; 2], [f32; 2])>,
}

#[derive(Clone, Debug, Default)]
pub struct BuildData {
    /// the registry's package
    pub pkg: String,
    /// keyed by MapBuildDataId digits ("C257E953...")
    pub meshes: HashMap<String, MeshBuildData>,
    /// light guid digits -> ShadowMapChannel (stationary lights)
    pub light_channels: HashMap<String, i32>,
    /// virtual texture indices per FLightMap2D found to fit (1 or 2)
    pub vt_indices: usize,
    /// LevelPrecomputedVolumetricLightmapBuildData: (level build guid, data); the persistent level's is the one drawn
    pub volumetric: Vec<(String, crate::vlm::VolumetricLightmap)>,
    /// ReflectionCaptureBuildData keyed by the capture component's MapBuildDataId digits
    pub reflections: HashMap<String, crate::vlm::ReflectionCapture>,
}

fn v2(r: &mut Cursor) -> [f32; 2] {
    [r.f32raw(), r.f32raw()]
}
fn v4(r: &mut Cursor) -> [f32; 4] {
    [r.f32raw(), r.f32raw(), r.f32raw(), r.f32raw()]
}
fn bool4(r: &mut Cursor) -> [bool; 4] {
    [r.s32() != 0, r.s32() != 0, r.s32() != 0, r.s32() != 0]
}
fn guids(r: &mut Cursor, end: i64) -> Option<Vec<String>> {
    let n = r.s32();
    if n < 0 || r.p + n as i64 * 16 > end {
        return None;
    }
    Some((0..n).map(|_| guid_digits(r)).collect())
}

struct Ctx<'a> {
    pk: &'a Pkgs,
    a: &'a std::rc::Rc<mh_pak::Package>,
}
impl Ctx<'_> {
    fn tex(&self, r: &mut Cursor) -> Option<TexRef> {
        let i = r.s32();
        let n = self.pk.rd.node(self.a, i)?;
        match &n {
            mh_pak::Node::Export(p, ix) => Some(TexRef { pkg: p.name.clone(), export: *ix, name: p.exports[*ix].name.clone() }),
            _ => Some(TexRef { pkg: String::new(), export: 0, name: self.pk.rd.node_name(&n) }),
        }
    }
    fn mesh(&self, r: &mut Cursor, end: i64, vt: usize) -> Option<MeshBuildData> {
        let mut m = MeshBuildData::default();
        match r.u32() {
            0 => {}
            2 => {
                let light_guids = guids(r, end)?;
                let textures = [self.tex(r), self.tex(r)];
                let sky_occlusion = self.tex(r);
                let ao_material_mask = self.tex(r);
                let mut scale_vectors = [[0f32; 4]; 4];
                let mut add_vectors = [[0f32; 4]; 4];
                for i in 0..4 {
                    scale_vectors[i] = v4(r);
                    add_vectors[i] = v4(r);
                }
                let coordinate_scale = v2(r);
                let coordinate_bias = v2(r);
                let shadow_channel_valid = bool4(r);
                let inv_uniform_penumbra_size = v4(r);
                r.skip(4 * vt as i64); // VirtualTextures
                m.light_map = Some(LightMap2D {
                    light_guids,
                    textures,
                    sky_occlusion,
                    ao_material_mask,
                    scale_vectors,
                    add_vectors,
                    coordinate_scale,
                    coordinate_bias,
                    shadow_channel_valid,
                    inv_uniform_penumbra_size,
                });
            }
            _ => return None, // LMT_1D is a pre-4.0 format
        }
        match r.u32() {
            0 => {}
            2 => {
                let light_guids = guids(r, end)?;
                let texture = self.tex(r);
                m.shadow_map = Some(ShadowMap2D {
                    light_guids,
                    texture,
                    coordinate_scale: v2(r),
                    coordinate_bias: v2(r),
                    channel_valid: bool4(r),
                    inv_uniform_penumbra_size: v4(r),
                });
            }
            _ => return None,
        }
        m.irrelevant_lights = guids(r, end)?;
        let (es, n) = (r.s32(), r.s32());
        if n < 0 || (n > 0 && es != 16) || r.p + n as i64 * 16 > end {
            return None;
        }
        m.per_instance = (0..n).map(|_| (v2(r), v2(r))).collect();
        (!r.bad && r.p <= end).then_some(m)
    }
}

/// The level's registry (Level.Properties.MapBuildData), None for an unbuilt level
pub fn read(pk: &Pkgs, level_pkg: &str) -> Option<BuildData> {
    let exps = pk.load_pkg(level_pkg);
    let lvl = exps.iter().find(|e| e.get("Type").and_then(|v| v.as_str()) == Some("Level"))?;
    let op = lvl.get("Properties")?.get("MapBuildData")?.get("ObjectPath")?.as_str()?;
    let (rpkg, ri) = crate::texture::split_ref(op)?;
    let a = pk.rd.open(rpkg)?;
    let cx = Ctx { pk, a: &a };
    for vt in [2usize, 1] {
        let (mut r, end) = after_props(pk, &a, ri)?;
        let (global, _class) = (r.u8(), r.u8());
        let mut out = BuildData { pkg: a.name.clone(), vt_indices: vt, ..Default::default() };
        if global & 2 != 0 {
            return Some(out);
        }
        let n = r.s32();
        if n < 0 || r.p + n as i64 * 20 > end {
            continue;
        }
        let mut ok = true;
        for _ in 0..n {
            let g = guid_digits(&mut r);
            match cx.mesh(&mut r, end, vt) {
                Some(m) => {
                    out.meshes.insert(g, m);
                }
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        // the next map's count must be sane, else the layout guess was wrong
        let nvol = r.peek_s32(0).unwrap_or(-1);
        if nvol < 0 || nvol > 4096 {
            continue;
        }
        // the rest of the registry: light volumes (skipped), volumetric lightmaps, light channels, reflection captures
        rest(&cx, &mut r, end, &mut out);
        return Some(out);
    }
    None
}

fn layer(a: &mh_pak::Package, r: &mut Cursor, end: i64) -> Option<crate::vlm::VlmLayer> {
    let c = r.s32();
    if c < 0 || r.p + c as i64 > end {
        return None;
    }
    let data = a.bytes_at(r.p, c as usize)?;
    r.skip(c as i64);
    Some(crate::vlm::VlmLayer { data, format: r.fstring() })
}

fn i3(r: &mut Cursor) -> [i32; 3] {
    [r.s32(), r.s32(), r.s32()]
}

/// After MeshBuildData (UMapBuildDataRegistry.cs:32-47): TMap<FGuid, FPrecomputedLightVolumeData> (skipped; :251-296
/// with FVolumeLightingSample :211-249), TMap<FGuid, FPrecomputedVolumetricLightmapData> (:298-369: bool bValid, FBox
/// Bounds, FIntVector IndirectionTextureDimensions, layer IndirectionTexture, int32 BrickSize, FIntVector
/// BrickDataDimensions, layers AmbientVector, SHCoefficients[6], SkyBentNormal, DirectionalLightShadowing, then
/// LQLightColor, LQLightDirection (FMobileObjectVersion LQVolumetricLightmapLayers), TArray<FIntVector>
/// SubLevelBrickPositions, TArray<FColor> IndirectionTextureOriginalValues (VolumetricLightmapStreaming)),
/// TMap<FGuid, FLightComponentMapBuildData> LightBuildData (:190-209), TMap<FGuid, FReflectionCaptureMapBuildData>
/// (:133-188: int32 CubemapSize, float AverageBrightness, float Brightness (StoreReflectionCaptureBrightnessForCooking),
/// TArray<uint8> FullHDRCapturedData, FPackageIndex EncodedCaptureData (StoreReflectionCaptureCompressedMobile)).
/// Each part is kept only if it parses to the end of its map; the checks are in tests/r4.rs against extract/json.
fn rest(cx: &Ctx, r: &mut Cursor, end: i64, out: &mut BuildData) -> Option<()> {
    let n = r.s32();
    for _ in 0..n.max(0) {
        r.skip(16);
        if r.s32() != 0 && r.s32() != 0 {
            r.skip(25 + 4);
            let nsh = r.s32();
            let order = if nsh == 9 { 3 } else { 2 };
            let sample = 12 + 4 + 3 * order * order * 4 + 4 + 4;
            for _ in 0..2 {
                let c = r.s32();
                if c < 0 || r.p + c as i64 * sample > end {
                    return None;
                }
                r.skip(c as i64 * sample);
            }
        }
    }
    let n = r.s32();
    for _ in 0..n.max(0) {
        let g = guid_digits(r);
        if r.s32() == 0 {
            continue;
        }
        let bounds_min = [r.f32raw() as f64, r.f32raw() as f64, r.f32raw() as f64];
        let bounds_max = [r.f32raw() as f64, r.f32raw() as f64, r.f32raw() as f64];
        r.u8(); // IsValid
        let ind = i3(r);
        let indirection = layer(cx.a, r, end)?;
        let brick_size = r.s32();
        let bd = i3(r);
        let ambient = layer(cx.a, r, end)?;
        let sh = [layer(cx.a, r, end)?, layer(cx.a, r, end)?, layer(cx.a, r, end)?, layer(cx.a, r, end)?, layer(cx.a, r, end)?, layer(cx.a, r, end)?];
        let sky_bent_normal = layer(cx.a, r, end)?;
        let directional_light_shadowing = layer(cx.a, r, end)?;
        let lq_light_color = layer(cx.a, r, end);
        let lq_light_direction = layer(cx.a, r, end);
        let c = r.s32();
        if c < 0 || r.p + c as i64 * 12 > end {
            return None;
        }
        let sub_level_brick_positions = (0..c).map(|_| i3(r)).collect();
        let c = r.s32();
        if c < 0 || r.p + c as i64 * 4 > end {
            return None;
        }
        let indirection_original_values = (0..c).map(|_| [r.u8(), r.u8(), r.u8(), r.u8()]).collect();
        let us = |x: [i32; 3]| x.map(|v| v.max(0) as usize);
        out.volumetric.push((g, crate::vlm::VolumetricLightmap {
            bounds_min,
            bounds_max,
            indirection_dims: us(ind),
            indirection,
            brick_size: brick_size.max(0) as usize,
            brick_dims: us(bd),
            ambient,
            sh,
            sky_bent_normal,
            directional_light_shadowing,
            lq_light_color,
            lq_light_direction,
            sub_level_brick_positions,
            indirection_original_values,
        }));
    }
    let n = r.s32();
    if !(0..=4096).contains(&n) {
        return None;
    }
    for _ in 0..n {
        let g = guid_digits(r);
        let ch = r.s32();
        r.skip(64 + 8); // FMatrix WorldToLight, ShadowMapSizeX/Y
        let c = r.s32();
        if c < 0 || r.p + c as i64 * 2 > end {
            return None;
        }
        r.skip(c as i64 * 2); // DepthSamples (FFloat16)
        out.light_channels.insert(g, ch);
    }
    let n = r.s32();
    if !(0..=65536).contains(&n) {
        return None;
    }
    for _ in 0..n {
        let g = guid_digits(r);
        let cubemap_size = r.s32().max(0) as usize;
        let average_brightness = r.f32raw();
        let brightness = r.f32raw();
        let c = r.s32();
        if c < 0 || r.p + c as i64 > end {
            return None;
        }
        let full_hdr = cx.a.bytes_at(r.p, c as usize)?;
        r.skip(c as i64);
        let encoded = cx.tex(r);
        out.reflections.insert(g, crate::vlm::ReflectionCapture { cubemap_size, average_brightness, brightness, full_hdr, encoded });
    }
    (!r.bad).then_some(())
}

//! uepost_render.rs - the GPU half of uepost.rs: UE 4.26's post chain as a Bevy Core3d render system (rust-render r1).
//! Runs in Core3dSystems::PostProcess before Bevy's tonemapping (which the camera sets to Tonemapping::None, so Bevy's
//! own pass is a no-op). Passes per view and frame (UE order, AddPostProcessingPasses 0x216d6d0; uepost.rs header):
//!   1. downsample scene -> D0 (1/2), D0 -> D1 (1/4), eye-adaptation setup on D1, D1 -> D2..D5 (1/8..1/64)
//!   2. eye adaptation on D5 -> EA[frame & 1] (1x1, previous = EA[!frame & 1])
//!   3. bloom: stage s = 0..5 (Bloom6..Bloom1) over D[5 - s]: horizontal blur -> T[s], vertical blur x tint + B[s-1] -> B[s]
//!   4. tonemap scene + B[5] + EA + LUT -> the view's next main texture (post_process_write)
//! The shader is uepost.wgsl. Texture formats: Rgba16Float throughout (UE: PF_FloatRGBA scene colour downsamples;
//! FRDGTextureDesc in AddDownsamplePass 0x2160d1d copies the input format).

use crate::uepost::{self, EyeParams};
use bevy::asset::uuid_handle;
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::core_pipeline::tonemapping::tonemapping;
use bevy::core_pipeline::FullscreenShader;
use bevy::prelude::*;
use bevy::render::camera::ExtractedCamera;
use bevy::render::extract_component::{ExtractComponent, ExtractComponentPlugin};
use bevy::render::extract_resource::{ExtractResource, ExtractResourcePlugin};
use bevy::render::render_asset::RenderAssets;
use bevy::render::render_resource::binding_types::{sampler, texture_2d, texture_3d, uniform_buffer_sized};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::sync_component::SyncComponent;
use bevy::render::texture::{CachedTexture, GpuImage, TextureCache};
use bevy::render::view::{ExtractedView, ViewTarget};
use bevy::render::{Render, RenderApp, RenderStartup, RenderSystems};
use bevy::shader::Shader;
use std::collections::HashMap;
use std::num::NonZeroU64;

pub const SHADER: Handle<Shader> = uuid_handle!("3b8e7c1a-5d24-4f0e-9a61-0c7d2e9b4f18");
const FMT: TextureFormat = TextureFormat::Rgba16Float;
const NV: usize = 72;

/// Debug switches for the per-stage evidence (script verb `post`): which UE stages run. Default: all.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Stages {
    pub eye_adaptation: bool,
    pub bloom: bool,
    pub film: bool,
    pub tint: bool,
    pub vignette: bool,
    pub sharpen: bool,
    pub fringe: bool,
}
impl Default for Stages {
    fn default() -> Self {
        Stages { eye_adaptation: true, bloom: true, film: true, tint: true, vignette: true, sharpen: true, fringe: true }
    }
}

/// On the camera: the level's post settings in GPU-ready form (built by camera.rs from lighting::Look).
#[derive(Component, Clone)]
pub struct UePostCam {
    pub lut: Handle<Image>,
    pub tint: [f32; 4],
    pub bloom_intensity: f32,
    pub vignette: f32,
    pub sharpen: f32,
    pub ca: [f32; 4],
    pub fringe: bool,
    /// Bloom1..6 KernelSizePercent (Size x BloomSizeScale) and tints (AddBloomPass 0x210c706-0x210c78b, tint x 1/6 at
    /// 0x210c8a8 with the vector 0.1666667 @0x45a4bf0)
    pub bloom_pct: [f32; 6],
    pub bloom_tint: [[f32; 4]; 6],
    pub eye: EyeParams,
    /// bumped on every level load: the eye adaptation jumps to its target (ForceTarget) for one frame
    pub cut: u32,
    /// debug: Some(e) = fixed exposure e instead of the adapted one (evidence of Bevy's old fixed exposure)
    pub fixed_exposure: Option<f32>,
    pub stages: Stages,
}

/// Debug state for the per-stage evidence (script verb `post`): applied to every UePostCam each frame.
#[derive(Resource, Clone, Copy, Default, Debug, serde::Serialize)]
pub struct PostDebug {
    pub stages: Stages,
    pub fixed_exposure: Option<f32>,
}

impl UePostCam {
    pub fn new(ps: &uepost::PostSettings, lut: Handle<Image>, cut: u32, dbg: &PostDebug) -> UePostCam {
        let mut pct = [0.0; 6];
        for i in 0..6 {
            pct[i] = ps.bloom_sizes[i] * ps.bloom_size_scale;
        }
        let mut tint = [[0.0; 4]; 6];
        for i in 0..6 {
            for j in 0..4 {
                tint[i][j] = ps.bloom_tints[i][j] * (1.0 / 6.0);
            }
        }
        UePostCam {
            lut,
            tint: ps.tint(),
            bloom_intensity: ps.bloom_intensity,
            vignette: ps.vignette_intensity,
            sharpen: uepost::sharpen_param(),
            ca: ps.ca_params(),
            fringe: ps.fringe_on(),
            bloom_pct: pct,
            bloom_tint: tint,
            eye: uepost::eye_params(ps),
            cut,
            fixed_exposure: dbg.fixed_exposure,
            stages: dbg.stages,
        }
    }
}

fn sync_debug(dbg: Res<PostDebug>, mut cams: Query<&mut UePostCam>) {
    if !dbg.is_changed() {
        return;
    }
    for mut c in cams.iter_mut() {
        c.stages = dbg.stages;
        c.fixed_exposure = dbg.fixed_exposure;
    }
}

impl SyncComponent for UePostCam {
    type Target = Self;
}
impl ExtractComponent for UePostCam {
    type QueryData = &'static UePostCam;
    type QueryFilter = ();
    type Out = Self;
    fn extract_component(c: &UePostCam) -> Option<Self> {
        Some(c.clone())
    }
}

/// World DeltaTime for the eye adaptation (UE: DeltaWorldTime, FEyeAdaptationParameters +0x18)
#[derive(Resource, Clone, Copy, Default, ExtractResource)]
pub struct UePostTime {
    pub dt: f32,
}

fn update_time(time: Res<Time>, mut t: ResMut<UePostTime>) {
    t.dt = time.delta_secs();
}

pub struct UePostPlugin;

impl Plugin for UePostPlugin {
    fn build(&self, app: &mut App) {
        // Our WGSL program is source code; the original cooked shader cache never ships.
        let shader = include_str!("uepost.wgsl");
        let _ = app
            .world_mut()
            .resource_mut::<Assets<Shader>>()
            .insert(SHADER.id(), Shader::from_wgsl(shader, "local-import/uepost.wgsl"));
        app.init_resource::<UePostTime>()
            .init_resource::<PostDebug>()
            .add_systems(Update, (update_time, sync_debug))
            .add_plugins((ExtractComponentPlugin::<UePostCam>::default(), ExtractResourcePlugin::<UePostTime>::default()));
        let Some(ra) = app.get_sub_app_mut(RenderApp) else { return };
        ra.init_resource::<EaStore>()
            .init_resource::<UiBlurR>()
            .add_systems(bevy::render::ExtractSchedule, extract_ui_blurs)
            .add_systems(RenderStartup, init_pipelines)
            .add_systems(Render, prepare_textures.in_set(RenderSystems::PrepareResources))
            .add_systems(Core3d, ue_post.before(tonemapping).in_set(Core3dSystems::PostProcess));
    }
}

#[derive(Resource)]
struct Pipes {
    layout: BindGroupLayoutDescriptor,
    sampler: Sampler,
    downsample: CachedRenderPipelineId,
    ea_setup: CachedRenderPipelineId,
    ea: CachedRenderPipelineId,
    blur: CachedRenderPipelineId,
    tonemap: CachedRenderPipelineId,
    copy: CachedRenderPipelineId,
    slate_down: CachedRenderPipelineId,
    slate_blur: CachedRenderPipelineId,
    slate_comp: CachedRenderPipelineId,
    dummy2d: TextureView,
    dummy3d: TextureView,
}

fn init_pipelines(mut commands: Commands, device: Res<RenderDevice>, cache: Res<PipelineCache>, fs: Res<FullscreenShader>) {
    let layout = BindGroupLayoutDescriptor::new(
        "ue_post_layout",
        &BindGroupLayoutEntries::sequential(
            ShaderStages::FRAGMENT,
            (
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_2d(TextureSampleType::Float { filterable: true }),
                texture_3d(TextureSampleType::Float { filterable: true }),
                sampler(SamplerBindingType::Filtering),
                uniform_buffer_sized(false, NonZeroU64::new((NV * 16) as u64)),
            ),
        ),
    );
    let sampler = device.create_sampler(&SamplerDescriptor {
        label: Some("ue_post_linear_clamp"),
        address_mode_u: AddressMode::ClampToEdge,
        address_mode_v: AddressMode::ClampToEdge,
        address_mode_w: AddressMode::ClampToEdge,
        mag_filter: FilterMode::Linear,
        min_filter: FilterMode::Linear,
        ..default()
    });
    let pipe = |entry: &'static str| {
        cache.queue_render_pipeline(RenderPipelineDescriptor {
            label: Some(format!("ue_post_{entry}").into()),
            layout: vec![layout.clone()],
            vertex: fs.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: SHADER,
                entry_point: Some(entry.into()),
                targets: vec![Some(ColorTargetState { format: FMT, blend: None, write_mask: ColorWrites::ALL })],
                ..default()
            }),
            ..default()
        })
    };
    let tex = |dim: TextureDimension, label: &'static str| {
        let t = device.create_texture(&TextureDescriptor {
            label: Some(label),
            size: Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: dim,
            format: FMT,
            usage: TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        t.create_view(&TextureViewDescriptor::default())
    };
    commands.insert_resource(Pipes {
        downsample: pipe("fs_downsample"),
        ea_setup: pipe("fs_ea_setup"),
        ea: pipe("fs_ea"),
        blur: pipe("fs_blur"),
        tonemap: pipe("fs_tonemap"),
        copy: pipe("fs_copy"),
        slate_down: pipe("fs_slate_down"),
        slate_blur: pipe("fs_slate_blur"),
        slate_comp: pipe("fs_slate_comp"),
        layout,
        sampler,
        dummy2d: tex(TextureDimension::D2, "ue_post_dummy2d"),
        dummy3d: tex(TextureDimension::D3, "ue_post_dummy3d"),
    });
}

/// Per-view intermediates: the downsample chain D0..D5, bloom temporaries T and results B (same sizes as D).
#[derive(Component)]
struct UePostTex {
    d: Vec<CachedTexture>,
    t: Vec<CachedTexture>,
    b: Vec<CachedTexture>,
}

/// The eye adaptation textures persist across frames (UE: FSceneViewState's eye adaptation render targets, swapped
/// every frame), keyed by the view's render entity; `last_cut` triggers ForceTarget.
#[derive(Resource, Default)]
struct EaStore(HashMap<Entity, Ea>);
struct Ea {
    tex: [Texture; 2],
    view: [TextureView; 2],
    frame: u64,
    last_cut: u32,
}

fn prepare_textures(
    mut commands: Commands,
    mut cache: ResMut<TextureCache>,
    device: Res<RenderDevice>,
    mut store: ResMut<EaStore>,
    views: Query<(Entity, &ExtractedCamera), With<UePostCam>>,
) {
    for (e, cam) in &views {
        let Some(size) = cam.physical_viewport_size else { continue };
        let mk = |cache: &mut TextureCache, w: u32, h: u32, label: &'static str| {
            cache.get(
                &device,
                TextureDescriptor {
                    label: Some(label),
                    size: Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: FMT,
                    usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                },
            )
        };
        // FIntPoint::DivideAndRoundUp(extent, 2) per stage (AddDownsamplePass)
        let mut sizes = Vec::new();
        let (mut w, mut h) = (size.x, size.y);
        for _ in 0..6 {
            w = w.div_ceil(2);
            h = h.div_ceil(2);
            sizes.push((w, h));
        }
        let d = sizes.iter().map(|&(w, h)| mk(&mut cache, w, h, "ue_post_down")).collect();
        let t = sizes.iter().map(|&(w, h)| mk(&mut cache, w, h, "ue_post_blur_h")).collect();
        let b = sizes.iter().map(|&(w, h)| mk(&mut cache, w, h, "ue_post_bloom")).collect();
        commands.entity(e).insert(UePostTex { d, t, b });
        store.0.entry(e).or_insert_with(|| {
            let mut tex = Vec::new();
            for _ in 0..2 {
                tex.push(device.create_texture(&TextureDescriptor {
                    label: Some("ue_post_eye_adaptation"),
                    size: Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: TextureDimension::D2,
                    format: FMT,
                    usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                }));
            }
            let view = [tex[0].create_view(&TextureViewDescriptor::default()), tex[1].create_view(&TextureViewDescriptor::default())];
            Ea { tex: [tex[0].clone(), tex[1].clone()], view, frame: 0, last_cut: u32::MAX }
        });
    }
}

fn ubuf(device: &RenderDevice, v: &[[f32; 4]]) -> Buffer {
    let mut bytes = vec![0u8; NV * 16];
    for (i, q) in v.iter().take(NV).enumerate() {
        for (j, x) in q.iter().enumerate() {
            bytes[i * 16 + j * 4..i * 16 + j * 4 + 4].copy_from_slice(&x.to_le_bytes());
        }
    }
    device.create_buffer_with_data(&BufferInitDescriptor { label: Some("ue_post_params"), contents: &bytes, usage: BufferUsages::UNIFORM })
}

#[allow(clippy::too_many_arguments)]
fn pass(
    ctx: &mut RenderContext,
    cache: &PipelineCache,
    pipes: &Pipes,
    pipe: &RenderPipeline,
    target: &TextureView,
    a: &TextureView,
    b: &TextureView,
    c: &TextureView,
    lut: &TextureView,
    params: &[[f32; 4]],
    label: &'static str,
) {
    let buf = ubuf(ctx.render_device(), params);
    let bg = ctx.render_device().create_bind_group(
        Some(label),
        &cache.get_bind_group_layout(&pipes.layout),
        &BindGroupEntries::sequential((a, b, c, lut, &pipes.sampler, buf.as_entire_binding())),
    );
    let mut rp = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: Operations { load: LoadOp::Clear(Default::default()), store: StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    rp.set_pipeline(pipe);
    rp.set_bind_group(0, &bg, &[]);
    rp.draw(0..3, 0..1);
}

#[allow(clippy::too_many_arguments)]
fn ue_post(
    view: ViewQuery<(Entity, &ViewTarget, &UePostCam, &UePostTex, &ExtractedView)>,
    pipes: Option<Res<Pipes>>,
    cache: Res<PipelineCache>,
    images: Res<RenderAssets<GpuImage>>,
    time: Option<Res<UePostTime>>,
    mut store: ResMut<EaStore>,
    blurs: Res<UiBlurR>,
    mut tcache: ResMut<TextureCache>,
    ecam: Query<&ExtractedCamera>,
    mut ctx: RenderContext,
) {
    let (entity, target, cam, tex, ev) = view.into_inner();
    let Some(pipes) = pipes else { return };
    if ev.target_format != FMT {
        return; // the pipelines are built for the HDR main texture (camera.rs sets Hdr)
    }
    let (Some(down), Some(setup), Some(eap), Some(blur), Some(tone), Some(_copy)) = (
        cache.get_render_pipeline(pipes.downsample),
        cache.get_render_pipeline(pipes.ea_setup),
        cache.get_render_pipeline(pipes.ea),
        cache.get_render_pipeline(pipes.blur),
        cache.get_render_pipeline(pipes.tonemap),
        cache.get_render_pipeline(pipes.copy),
    ) else {
        return;
    };
    let Some(lut) = images.get(&cam.lut) else { return };
    let Some(ea) = store.0.get_mut(&entity) else { return };
    let dt = time.map(|t| t.dt).unwrap_or(1.0 / 60.0);
    let force = if ea.last_cut != cam.cut || ea.frame == 0 { 1.0 } else { 0.0 };
    ea.last_cut = cam.cut;
    let cur = (ea.frame & 1) as usize;
    ea.frame += 1;
    let (z2, z3) = (&pipes.dummy2d, &pipes.dummy3d);
    let pw = target.post_process_write();
    let scene = pw.source;
    let ext = pw.source_texture.size();
    // 1. downsample chain + eye adaptation setup at 1/4 (FSceneDownsampleChain::Init 0x217d25c-0x217d290)
    let e = &cam.eye;
    let texel = |t: &Texture| [1.0 / t.width() as f32, 1.0 / t.height() as f32, 0.0, 0.0];
    let (sw, sh) = (ext.width as f32, ext.height as f32);
    pass(&mut ctx, &cache, &pipes, down, &tex.d[0].default_view, scene, z2, z2, z3, &[[1.0 / sw, 1.0 / sh, 0.0, 0.0]], "ue_post_down0");
    for i in 1..6 {
        let src = &tex.d[i - 1];
        if i == 2 {
            // D1 holds plain colour; the setup pass rewrites it with log luminance in alpha (into T1, then T1 is the
            // chain's stage 1 for the next halving; same size)
            pass(&mut ctx, &cache, &pipes, setup, &tex.t[1].default_view, &tex.d[1].default_view, z2, z2, z3,
                &[[e.hist_scale, e.hist_bias, e.lum_min, 0.0]], "ue_post_ea_setup");
            pass(&mut ctx, &cache, &pipes, down, &tex.d[i].default_view, &tex.t[1].default_view, z2, z2, z3, &[texel(&tex.t[1].texture)], "ue_post_down");
            continue;
        }
        pass(&mut ctx, &cache, &pipes, down, &tex.d[i].default_view, &src.default_view, z2, z2, z3, &[texel(&src.texture)], "ue_post_down");
    }
    // 2. eye adaptation
    let prev = &ea.view[1 - cur];
    let ea_out = &ea.view[cur];
    let ea_params = [
        [e.min_avg_lum, e.max_avg_lum, e.comp_settings * e.comp_curve, e.grey_mult],
        [dt, e.speed_up, e.speed_down, force],
        [e.hist_scale, e.hist_bias, e.exp_up_m, e.exp_down_m],
        [e.start_distance, 1.0, 0.0, 0.0],
    ];
    if cam.stages.eye_adaptation && cam.fixed_exposure.is_none() {
        pass(&mut ctx, &cache, &pipes, eap, ea_out, &tex.d[5].default_view, prev, z2, z3, &ea_params, "ue_post_eye_adaptation");
    } else {
        // fixed exposure: the 1x1 target cleared to (e, e, 0, 1)
        let fx = cam.fixed_exposure.unwrap_or(1.0);
        let _rp = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
            label: Some("ue_post_fixed_exposure"),
            color_attachments: &[Some(RenderPassColorAttachment {
                view: ea_out,
                depth_slice: None,
                resolve_target: None,
                ops: Operations { load: LoadOp::Clear(bevy::color::LinearRgba::rgb(fx, fx, 0.0).into()), store: StoreOp::Store },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
    }
    // 3. bloom (AddBloomPass 0x210c7c7-0x210c8f2): Bloom6 over D5 first, each stage adds the previous one
    let mut last: Option<usize> = None;
    if cam.stages.bloom {
        for s in 0..6 {
            let size_idx = 5 - s; // Bloom6..Bloom1
            let pct = cam.bloom_pct[size_idx];
            if pct <= 1e-8 {
                continue;
            }
            let src = &tex.d[5 - s];
            let (w, h) = (src.texture.width() as f32, src.texture.height() as f32);
            // KernelRadius = viewport width x KernelSizePercent x 0.005 for both directions (AddGaussianBlurPass 0x21acff0-
            // 0x21ad00e; the vertical kernel reuses xmm8 at 0x21ad45b); CrossCenterWeight 0 (r.Bloom.Cross default 0)
            let k = uepost::gauss_kernel(w * pct * 0.005, uepost::MAX_FILTER_SAMPLES, 0.0);
            let mut hp = vec![[1.0 / w, 0.0, k.len() as f32, 0.0], [1.0, 1.0, 1.0, 1.0]];
            hp.extend(k.iter().map(|&(o, wt)| [o, wt, 0.0, 0.0]));
            let ti = 5 - s; // temporaries sized like the stage's input
            pass(&mut ctx, &cache, &pipes, blur, &tex.t[ti].default_view, &src.default_view, z2, z2, z3, &hp, "ue_post_bloom_h");
            let mut vp = vec![[0.0, 1.0 / h, k.len() as f32, if last.is_some() { 1.0 } else { 0.0 }], cam.bloom_tint[size_idx]];
            vp.extend(k.iter().map(|&(o, wt)| [o, wt, 0.0, 0.0]));
            let add = last.map(|l| &tex.b[l].default_view).unwrap_or(z2);
            pass(&mut ctx, &cache, &pipes, blur, &tex.b[ti].default_view, &tex.t[ti].default_view, add, z2, z3, &vp, "ue_post_bloom_v");
            last = Some(ti);
        }
    }
    // 4. tonemap
    let bloom_view = last.map(|l| &tex.b[l].default_view).unwrap_or(z2);
    let st = cam.stages;
    let tint = if st.tint { cam.tint } else { [1.0; 4] };
    let noise = (ea.frame % 4096) as f32 * 0.000244;
    let tp = [
        tint,
        [if st.bloom { cam.bloom_intensity } else { 0.0 }, if st.vignette { cam.vignette } else { 0.0 }, if st.sharpen { cam.sharpen } else { 0.0 }, 1.0],
        [cam.ca[0], cam.ca[1], cam.ca[2], if st.fringe && cam.fringe { 1.0 } else { 0.0 }],
        [1.0 / sw, 1.0 / sh, sh / sw, noise],
        [sw, sh, if st.film { 1.0 } else { 0.0 }, 0.0],
    ];
    pass(&mut ctx, &cache, &pipes, tone, pw.destination, scene, bloom_view, ea_out, &lut.texture_view, &tp, "ue_post_tonemap");
    // 5. UMG BackgroundBlur elements over the finished scene (Bevy UI draws after this system)
    if !blurs.els.is_empty() {
        let phys = ecam.get(entity).ok().and_then(|c| c.physical_viewport_size).unwrap_or(bevy::math::UVec2::new(ext.width, ext.height));
        ui_blur(&mut ctx, &cache, &pipes, target, &blurs, phys, &mut tcache);
    }
}

/// The frame's UBackgroundBlur elements (mh_ui::slate::UiBlurs), render-world copy
#[derive(Resource, Default)]
pub struct UiBlurR {
    els: Vec<mh_ui::slate::BlurEl>,
    size: [f64; 2],
}

fn extract_ui_blurs(mut r: ResMut<UiBlurR>, b: bevy::render::Extract<Option<Res<mh_ui::slate::UiBlurs>>>) {
    match b.as_ref() {
        Some(b) => {
            r.els.clone_from(&b.els);
            r.size = b.size;
        }
        None => r.els.clear(),
    }
}

/// Slate's post-process blur (FSlatePostProcessor::BlurRect rva 0x2b73230) for each BackgroundBlur element, in paint
/// order: the element's rect of the current image is downsampled (FSlatePostProcessDownsamplePS) into a
/// DivideAndRoundUp(size, Downsample) target, blurred horizontally then vertically (FSlatePostProcessBlurPS with the
/// weight table of uepost::slate_blur_weights) and drawn back over the rect. Only the 3D scene lies under it here:
/// UI items painted before the blur widget are drawn later by Bevy UI and stay sharp (UNCONFIRMED difference: Slate
/// blurs whatever the back buffer holds, earlier widgets included).
#[allow(clippy::too_many_arguments)]
fn ui_blur(ctx: &mut RenderContext, cache: &PipelineCache, pipes: &Pipes, target: &ViewTarget, b: &UiBlurR, phys: bevy::math::UVec2,
    tcache: &mut TextureCache) {
    let (Some(down), Some(blur), Some(comp), Some(copy)) = (
        cache.get_render_pipeline(pipes.slate_down),
        cache.get_render_pipeline(pipes.slate_blur),
        cache.get_render_pipeline(pipes.slate_comp),
        cache.get_render_pipeline(pipes.copy),
    ) else {
        return;
    };
    let (z2, z3) = (&pipes.dummy2d, &pipes.dummy3d);
    // a new main texture holding the tonemapped image, which the elements then blur in place
    let pw = target.post_process_write();
    pass(ctx, cache, pipes, copy, pw.destination, pw.source, z2, z2, z3, &[[1.0, 0.0, 0.0, 0.0]], "ue_ui_blur_copy");
    let dst = pw.destination;
    let dst_tex = pw.destination_texture;
    let (vw, vh) = (phys.x as f32, phys.y as f32);
    let k = if b.size[0] > 0.0 { vw / b.size[0] as f32 } else { 1.0 };
    let device = ctx.render_device().clone();
    for el in &b.els {
        // the element's render-bounding rect in target pixels, clipped to its clip rect and the view
        let mut r = [el.rect[0] as f32 * k, el.rect[1] as f32 * k, (el.rect[0] + el.rect[2]) as f32 * k, (el.rect[1] + el.rect[3]) as f32 * k];
        if let Some(c) = el.clip {
            r = [r[0].max(c[0] as f32 * k), r[1].max(c[1] as f32 * k), r[2].min((c[0] + c[2]) as f32 * k), r[3].min((c[1] + c[3]) as f32 * k)];
        }
        r = [r[0].max(0.0), r[1].max(0.0), r[2].min(vw), r[3].min(vh)];
        let (w, h) = (r[2] - r[0], r[3] - r[1]);
        if w < 1.0 || h < 1.0 {
            continue;
        }
        let Some((kernel, sigma, ds, rw, rh)) = uepost::slate_blur_params(el.strength as f32, el.radius, w, h) else { continue };
        let mut mk = |label: &'static str| {
            tcache.get(&device, TextureDescriptor {
                label: Some(label),
                size: Extent3d { width: rw as u32, height: rh as u32, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format: FMT,
                usage: TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
        };
        let ta = mk("ue_ui_blur_a");
        let tb = mk("ue_ui_blur_b");
        // downsample / copy the rect (UV bounds = the rect, half a texel in)
        let (sw, sh) = (dst_tex.width() as f32, dst_tex.height() as f32);
        let uvr = [r[0] / sw, r[1] / sh, r[2] / sw, r[3] / sh];
        let bounds = [(r[0] + 0.5) / sw, (r[1] + 0.5) / sh, (r[2] - 0.5) / sw, (r[3] - 0.5) / sh];
        // tap offset: one source texel (DownsampleRect's InvSrcTextureSize, recalled from UE 4.26
        // SlatePostProcessor.cpp: UNCONFIRMED in the exe)
        let dp = [[1.0 / sw, 1.0 / sh, 0.0, 0.0], uvr, bounds, [if ds > 0 { 1.0 } else { 0.0 }, 0.0, 0.0, 0.0]];
        pass(ctx, cache, pipes, down, &ta.default_view, dst, z2, z2, z3, &dp, "ue_ui_blur_down");
        // horizontal then vertical blur over the small target
        let (wt, n) = uepost::slate_blur_weights(kernel, sigma);
        let inv = [1.0 / rw as f32, 1.0 / rh as f32];
        for (dir, from, to) in [([1.0f32, 0.0f32], &ta, &tb), ([0.0, 1.0], &tb, &ta)] {
            let mut v = vec![[0.0f32; 4]; 66];
            for (i, e) in wt.iter().take(63).enumerate() {
                v[i] = *e;
            }
            v[63] = [n as f32, 0.0, 0.0, 0.0];
            v[64] = [dir[0], dir[1], inv[0], inv[1]];
            v[65] = [0.0, 0.0, 1.0, 1.0];
            pass(ctx, cache, pipes, blur, &to.default_view, &from.default_view, z2, z2, z3, &v, "ue_ui_blur");
        }
        // back over the rect
        comp_pass(ctx, cache, pipes, comp, dst, &ta.default_view, r, "ue_ui_blur_comp");
    }
}

/// A pass that keeps the target's contents and draws only inside `r` (target pixels: x0, y0, x1, y1)
#[allow(clippy::too_many_arguments)]
fn comp_pass(ctx: &mut RenderContext, cache: &PipelineCache, pipes: &Pipes, pipe: &RenderPipeline, target: &TextureView, a: &TextureView,
    r: [f32; 4], label: &'static str) {
    let buf = ubuf(ctx.render_device(), &[]);
    let bg = ctx.render_device().create_bind_group(
        Some(label),
        &cache.get_bind_group_layout(&pipes.layout),
        &BindGroupEntries::sequential((a, &pipes.dummy2d, &pipes.dummy2d, &pipes.dummy3d, &pipes.sampler, buf.as_entire_binding())),
    );
    let mut rp = ctx.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: Operations { load: LoadOp::Load, store: StoreOp::Store },
        })],
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    rp.set_pipeline(pipe);
    rp.set_bind_group(0, &bg, &[]);
    rp.set_viewport(r[0], r[1], r[2] - r[0], r[3] - r[1], 0.0, 1.0);
    rp.draw(0..3, 0..1);
}


/// The LUT volume as a Bevy image (Rgba16Float 32^3; uepost::combined_lut, r fastest)
pub fn lut_image(lut: &[[f32; 3]]) -> Image {
    let n = uepost::LUT_SIZE as u32;
    let mut data = Vec::with_capacity(lut.len() * 8);
    for c in lut {
        for x in [c[0], c[1], c[2], 1.0] {
            data.extend_from_slice(&f32_to_f16(x).to_le_bytes());
        }
    }
    let mut img = Image::new(Extent3d { width: n, height: n, depth_or_array_layers: n }, TextureDimension::D3, data, FMT,
        bevy::asset::RenderAssetUsages::RENDER_WORLD);
    img.sampler = crate::paksrc::clamp_sampler();
    img
}

/// IEEE half from f32 (round to nearest even, finite inputs in [0, 2])
pub fn f32_to_f16(x: f32) -> u16 {
    let b = x.to_bits();
    let sign = ((b >> 16) & 0x8000) as u16;
    let exp = ((b >> 23) & 0xff) as i32 - 127 + 15;
    let mant = b & 0x7f_ffff;
    if exp <= 0 {
        if exp < -10 {
            return sign;
        }
        let m = (mant | 0x80_0000) >> (1 - exp);
        let r = (m >> 13) + (((m >> 12) & 1) & (((m & 0xfff) != 0) as u32 | ((m >> 13) & 1)));
        return sign | r as u16;
    }
    if exp >= 31 {
        return sign | 0x7c00;
    }
    let mut h = ((exp as u32) << 10) | (mant >> 13);
    let rem = mant & 0x1fff;
    if rem > 0x1000 || (rem == 0x1000 && (h & 1) == 1) {
        h += 1;
    }
    sign | h as u16
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn half_round_trip() {
        for x in [0.0f32, 0.5, 1.0, 0.952381, 0.001, 1.05] {
            let h = f32_to_f16(x);
            let e = ((h >> 10) & 0x1f) as i32;
            let m = (h & 0x3ff) as f32;
            let v = if e == 0 { m * 2f32.powi(-24) } else { (1.0 + m / 1024.0) * 2f32.powi(e - 15) };
            assert!((v - x).abs() <= x.abs() * 1e-3 + 1e-6, "{x} -> {v}");
        }
    }
}

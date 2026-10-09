//! Offscreen evidence harness for the plugin crates' examples (mh-ui, mh-fx, mh-audio): a windowless Bevy app whose
//! camera renders into an image (as mh-runtime's --offscreen), a run directory
//! state/runtime_evidence/<date>/<time>-<name>/ (docs/RUST_RUNTIME.md section 3 layout), screenshots of the image.

use bevy::app::ScheduleRunnerPlugin;
use bevy::prelude::*;
use bevy::render::view::window::screenshot::{save_to_disk, Screenshot};
use bevy::window::{ExitCondition, WindowPlugin};
use std::path::PathBuf;
use std::time::Duration;

pub const SIZE: (u32, u32) = (1280, 720);

/// the offscreen image the camera renders into
#[derive(Resource, Clone)]
pub struct OffscreenImage(pub Handle<Image>);

/// the evidence camera
#[derive(Resource, Clone, Copy)]
pub struct EvidenceCamera(pub Entity);

/// state/runtime_evidence/<YYYY-MM-DD>/<HHMMSS>-<name>/ under the repo root (UTC)
pub fn run_dir(name: &str) -> PathBuf {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) as i64;
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    // civil date from days since 1970-01-01
    let z = days + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../state/runtime_evidence")
        .join(format!("{y:04}-{m:02}-{d:02}"))
        .join(format!("{:02}{:02}{:02}-{name}", rem / 3600, rem % 3600 / 60, rem % 60));
    let _ = std::fs::create_dir_all(&dir);
    dir
}

/// A windowless app rendering offscreen (GPU) through a Camera3d (`three_d`) or Camera2d, the UI targeted at it
pub fn offscreen_app(three_d: bool) -> App {
    offscreen_app_sized(three_d, SIZE)
}

/// offscreen_app at a given image size (the window size being emulated)
pub fn offscreen_app_sized(three_d: bool, size_px: (u32, u32)) -> App {
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin { primary_window: None, exit_condition: ExitCondition::DontExit, ..default() })
            .set(bevy::log::LogPlugin { filter: "wgpu=error,naga=warn".into(), ..default() })
            .disable::<bevy::winit::WinitPlugin>(),
    )
    .add_plugins(ScheduleRunnerPlugin::run_loop(Duration::from_secs_f64(1.0 / 60.0)));
    use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
    let size = Extent3d { width: size_px.0, height: size_px.1, depth_or_array_layers: 1 };
    let mut img = Image::new_fill(size, TextureDimension::D2, &[0, 0, 0, 255], TextureFormat::Rgba8UnormSrgb, bevy::asset::RenderAssetUsages::default());
    img.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST | TextureUsages::COPY_SRC | TextureUsages::RENDER_ATTACHMENT;
    let h = app.world_mut().resource_mut::<Assets<Image>>().add(img);
    app.insert_resource(OffscreenImage(h.clone()));
    let target = bevy::camera::RenderTarget::Image(h.into());
    let cam = if three_d {
        app.world_mut().spawn((Camera3d::default(), target, Transform::from_xyz(0.0, 1.2, 3.0).looking_at(Vec3::new(0.0, 1.0, 0.0), Vec3::Y))).id()
    } else {
        app.world_mut().spawn((Camera2d, target)).id()
    };
    app.insert_resource(EvidenceCamera(cam));
    app.insert_resource(crate::UiTarget(cam));
    app.insert_resource(crate::UiViewport(Vec2::new(size_px.0 as f32, size_px.1 as f32)));
    app
}

/// request a screenshot of the offscreen image into `path` (written a few frames later)
pub fn screenshot(world: &mut World, path: PathBuf) {
    let h = world.resource::<OffscreenImage>().0.clone();
    world.spawn(Screenshot(bevy::camera::RenderTarget::Image(h.into()))).observe(save_to_disk(path));
}

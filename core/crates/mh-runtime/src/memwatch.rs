//! Memory watch (diagnostics, not game behaviour): once per real second, the entity count, the Assets<T> counts of
//! every asset type the runtime and its plugins create at run time, and the process's private bytes. dump_state
//! carries the rows ("memwatch"); `memcheck <window_s> <max_mb_per_s> [<max_asset_growth>]` (script.rs) fails a run whose private
//! bytes or asset counts keep growing over the trailing window (the per-frame leak regression check, 2026-10-06).

use bevy::prelude::*;
use std::time::Instant;

#[derive(Resource, Clone, Debug, serde::Serialize)]
pub struct MemWatch {
    #[serde(skip)]
    pub last: Option<Instant>,
    #[serde(skip)]
    pub t0: Option<Instant>,
    pub rows: Vec<MemRow>,
    /// rows are also printed to stderr (MH_MEMLOG=1)
    pub print: bool,
    /// this second's frame times (s), real time between `Last` runs (first-person r1: smoothness numbers)
    #[serde(skip)]
    pub dts: Vec<f32>,
    #[serde(skip)]
    pub prev_frame: Option<Instant>,
}

impl Default for MemWatch {
    fn default() -> Self {
        MemWatch { last: None, t0: None, rows: Vec::new(), print: std::env::var("MH_MEMLOG").is_ok_and(|v| v == "1"), dts: Vec::new(), prev_frame: None }
    }
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct MemRow {
    pub secs: f32,
    pub frame: u64,
    pub private_mb: f64,
    pub entities: u32,
    pub assets: Vec<(&'static str, usize)>,
    /// frame time over the second (ms): min / avg / p99 / max
    pub frame_ms: [f32; 4],
}

impl MemRow {
    pub fn asset_total(&self) -> usize {
        self.assets.iter().map(|a| a.1).sum()
    }
}

pub struct MemWatchPlugin;

impl Plugin for MemWatchPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MemWatch>().add_systems(Last, sample);
    }
}

fn count<T: Asset>(world: &World) -> usize {
    world.get_resource::<Assets<T>>().map(|a| a.len()).unwrap_or(0)
}

/// the process's private (commit) bytes in MB (Windows: PROCESS_MEMORY_COUNTERS_EX.PrivateUsage)
#[cfg(windows)]
pub fn private_mb() -> f64 {
    #[repr(C)]
    #[allow(non_snake_case)]
    struct Pmc {
        cb: u32,
        PageFaultCount: u32,
        PeakWorkingSetSize: usize,
        WorkingSetSize: usize,
        QuotaPeakPagedPoolUsage: usize,
        QuotaPagedPoolUsage: usize,
        QuotaPeakNonPagedPoolUsage: usize,
        QuotaNonPagedPoolUsage: usize,
        PagefileUsage: usize,
        PeakPagefileUsage: usize,
        PrivateUsage: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(h: isize, p: *mut Pmc, cb: u32) -> i32;
    }
    let mut p = Pmc { cb: std::mem::size_of::<Pmc>() as u32, PageFaultCount: 0, PeakWorkingSetSize: 0, WorkingSetSize: 0, QuotaPeakPagedPoolUsage: 0, QuotaPagedPoolUsage: 0, QuotaPeakNonPagedPoolUsage: 0, QuotaNonPagedPoolUsage: 0, PagefileUsage: 0, PeakPagefileUsage: 0, PrivateUsage: 0 };
    // SAFETY: a plain Win32 query into a correctly sized, initialised struct
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut p, p.cb) };
    if ok == 0 {
        return 0.0;
    }
    p.PrivateUsage as f64 / (1024.0 * 1024.0)
}

#[cfg(not(windows))]
pub fn private_mb() -> f64 {
    0.0
}

pub fn row(world: &World, secs: f32, frame: u64) -> MemRow {
    let assets = vec![
        ("Mesh", count::<Mesh>(world)),
        ("Image", count::<Image>(world)),
        ("StandardMaterial", count::<StandardMaterial>(world)),
        ("UeTintMaterial", count::<crate::uetint::UeTintMaterial>(world)),
        ("ParticleMaterial", count::<mh_fx::material::ParticleMaterial>(world)),
        ("UeParticleMaterial", count::<mh_fx::ue_material::UeParticleMaterial>(world)),
        ("CueVoice", count::<mh_audio::CueVoice>(world)),
        ("AnimationClip", count::<AnimationClip>(world)),
        ("AnimationGraph", count::<AnimationGraph>(world)),
        ("SkinnedMeshInverseBindposes", count::<bevy::mesh::skinning::SkinnedMeshInverseBindposes>(world)),
    ];
    MemRow { secs, frame, private_mb: private_mb(), entities: world.entities().len(), assets, frame_ms: [0.0; 4] }
}

/// min / avg / p99 / max of frame times (s) in ms
pub fn frame_stats(dts: &[f32]) -> [f32; 4] {
    if dts.is_empty() {
        return [0.0; 4];
    }
    let mut v: Vec<f32> = dts.iter().map(|d| d * 1000.0).collect();
    v.sort_by(|a, b| a.total_cmp(b));
    let p99 = v[((v.len() as f32 * 0.99).ceil() as usize).clamp(1, v.len()) - 1];
    [v[0], v.iter().sum::<f32>() / v.len() as f32, p99, v[v.len() - 1]]
}

fn sample(world: &mut World) {
    let now = Instant::now();
    {
        let mut w = world.resource_mut::<MemWatch>();
        if let Some(p) = w.prev_frame {
            let d = now.duration_since(p).as_secs_f32();
            w.dts.push(d);
        }
        w.prev_frame = Some(now);
    }
    let (due, t0) = {
        let w = world.resource::<MemWatch>();
        (w.last.is_none_or(|l| now.duration_since(l).as_secs_f32() >= 1.0), w.t0.unwrap_or(now))
    };
    if !due {
        return;
    }
    let frame = world.get_resource::<crate::script::Script>().map(|s| s.frame).unwrap_or(0);
    let mut r = row(world, now.duration_since(t0).as_secs_f32(), frame);
    let mut w = world.resource_mut::<MemWatch>();
    r.frame_ms = frame_stats(&w.dts);
    w.dts.clear();
    if w.print {
        eprintln!("memwatch {:.0}s frame {} private {:.0} MB entities {} frame_ms {:?} {:?}", r.secs, r.frame, r.private_mb, r.entities, r.frame_ms, r.assets);
    }
    w.t0 = Some(t0);
    w.last = Some(now);
    w.rows.push(r);
    // a long windowed session: keep the first minute and the last ten
    if w.rows.len() > 120 {
        w.rows.remove(60);
    }
}

/// the leak check over the trailing `window` seconds of rows: (private MB growth per second, per-asset-type growth)
pub fn growth(rows: &[MemRow], window: f32) -> Option<(f64, Vec<(&'static str, i64)>)> {
    let end = rows.last()?.secs;
    let after: Vec<&MemRow> = rows.iter().filter(|r| r.secs >= end - window).collect();
    let (a, b) = (after.first()?, after.last()?);
    let dt = (b.secs - a.secs) as f64;
    if dt < 2.0 {
        return None;
    }
    let per_s = (b.private_mb - a.private_mb) / dt;
    let d = a.assets.iter().zip(b.assets.iter()).map(|(x, y)| (x.0, y.1 as i64 - x.1 as i64)).collect();
    Some((per_s, d))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(secs: f32, mb: f64, meshes: usize) -> MemRow {
        MemRow { secs, frame: 0, private_mb: mb, entities: 0, assets: vec![("Mesh", meshes)], frame_ms: [0.0; 4] }
    }

    #[test]
    fn growth_after_warmup() {
        let rows = vec![r(0.0, 100.0, 1), r(5.0, 500.0, 50), r(6.0, 500.0, 50), r(16.0, 800.0, 950)];
        let (per_s, d) = growth(&rows, 11.0).unwrap();
        assert!((per_s - 300.0 / 11.0).abs() < 1e-9);
        assert_eq!(d, vec![("Mesh", 900)]);
        assert!(growth(&rows[..3], 0.5).is_none());
    }

    #[test]
    fn private_bytes_readable() {
        #[cfg(windows)]
        assert!(private_mb() > 1.0);
    }
}

#[cfg(test)]
mod frame_tests {
    #[test]
    fn frame_stats_min_avg_p99_max() {
        let mut d: Vec<f32> = (1..=100).map(|i| i as f32 / 1000.0).collect();
        d.reverse();
        let s = super::frame_stats(&d);
        assert_eq!(s[0], 1.0);
        assert!((s[1] - 50.5).abs() < 1e-3);
        assert_eq!(s[2], 99.0);
        assert_eq!(s[3], 100.0);
        assert_eq!(super::frame_stats(&[]), [0.0; 4]);
    }
}

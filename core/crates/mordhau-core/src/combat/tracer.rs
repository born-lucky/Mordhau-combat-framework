//! The melee weapon's hit trace: AMordhauWeapon's tracer state and its sampling (godot/game/combat/weapon_tracer.gd;
//! extract/native/decomp/AMordhauWeapon.cpp, fields types/AMordhauWeapon.h):
//!   PrepareForTracing rva=0x16378e0, ResetTracers rva=0x163bb80, GetTrace_Implementation rva=0x1629520,
//!   SampleTracers rva=0x163c430 (sample count = RoundToInt(len_cm x 2/7.5 + 0.5) >> 1, segments every 7.5 cm from
//!   the TraceStart side to TraceEnd, each swept from last tick to now), RecalculateTracerPoints rva=0x163a940 (Length).
//! Positions are host world space in metres (the Godot convention of the reference: UE cm x 0.01); f32 vectors.

use crate::data::{Constants, TracerDef};
use crate::ue::{cvtss2si, FVector, Xform};

const CM_PER_M: f64 = 100.0; // sockets are stored in metres (glTF writer: UE cm x 0.01); UE math is in cm

#[derive(Clone, Debug, Default)]
pub struct Tracer {
    pub trace_start_socket: Option<FVector>, // None = the mesh has no such socket
    pub trace_end_socket: Option<FVector>,
    pub cur_start: FVector,         // +0xd48 CurrentTraceStart
    pub cur_end: FVector,           // +0xd54 CurrentTraceEnd
    pub prev_start: FVector,        // +0xd60 PreviousTraceStart
    pub prev_end: FVector,          // +0xd6c PreviousTraceEnd
    pub b_cur_valid: bool,          // +0xd40 bAreCurrentTracersValid
    pub b_prev_valid: bool,         // +0xd41 bArePreviousTracersValid
    pub b_cur_invalidated: bool,    // +0xd42 bAreCurrentTracersInvalidated
    pub root_xf: Xform,             // world transform of the socket bone, fed each tick by the host
    pub actor_ignore_cache: Vec<u32>, // +0xe88 ActorIgnoreCache (Fighter::id)
    pub length: f64,                // +0x1bec Length
    pub count_scale: f64,           // Constants::tracer_count_scale (.rdata 0x144362ca8)
    pub round_bias: f64,            // Constants::tracer_round_bias (.rdata 0x143fe4e04)
    pub spacing_cm: f64,            // Constants::tracer_spacing_cm (.rdata 0x144362cc0)
    pub step: f64,                  // Constants::tracer_step (.rdata 0x144014bb8)
}

impl Tracer {
    pub fn new(d: &TracerDef, c: &Constants) -> Tracer {
        let mut t = Tracer {
            trace_start_socket: d.start,
            trace_end_socket: d.end,
            root_xf: Xform::IDENTITY,
            count_scale: c.tracer_count_scale,
            round_bias: c.tracer_round_bias,
            spacing_cm: c.tracer_spacing_cm,
            step: c.tracer_step,
            ..Default::default()
        };
        // RecalculateTracerPoints rva=0x163a940: Length = |TraceEnd.Z - TraceStart.Z| (UE cm) x 1/15; UE Z = host Y
        if let (Some(a), Some(b)) = (d.start, d.end) {
            let span_m = (b.y as f64 - a.y as f64).abs();
            t.length = span_m * CM_PER_M * c.tracer_length_per_cm;
        }
        t
    }

    fn socket_world(&self, s: Option<FVector>) -> FVector {
        self.root_xf.apply(s.unwrap_or(FVector::ZERO))
    }

    /// from AMordhauWeapon::ResetTracers rva=0x163bb80
    pub fn reset_tracers(&mut self) {
        self.b_cur_invalidated = true;
    }

    /// from AMordhauWeapon::PrepareForTracing rva=0x16378e0
    pub fn prepare_for_tracing(&mut self) {
        self.prev_start = self.cur_start;
        self.prev_end = self.cur_end;
        self.cur_start = self.socket_world(self.trace_start_socket);
        self.cur_end = self.socket_world(self.trace_end_socket);
        let was = self.b_cur_invalidated;
        self.b_prev_valid = self.b_cur_valid;
        if was {
            self.b_cur_invalidated = false;
        }
        self.b_cur_valid = !was;
    }

    /// from AMordhauWeapon::SampleTracers rva=0x163c430: [(from, to)], i = n (base side) first, TraceEnd last
    pub fn segments(&self) -> Vec<(FVector, FVector)> {
        if !(self.b_cur_valid && self.b_prev_valid) {
            return Vec::new();
        }
        let dir_c = (self.cur_start - self.cur_end).normalized();
        let dir_p = (self.prev_start - self.prev_end).normalized();
        let len_cm = (self.cur_end - self.cur_start).length() as f64 * 100.0;
        let n = cvtss2si(len_cm * self.count_scale + self.round_bias) >> 1;
        let step = self.spacing_cm * 0.01; // 7.5 cm in metres
        let mut out = Vec::new();
        let mut i = n as f64;
        while i >= 0.0 {
            out.push((self.prev_end + dir_p.scale(i).scale(step), self.cur_end + dir_c.scale(i).scale(step)));
            i += self.step;
        }
        out
    }
}

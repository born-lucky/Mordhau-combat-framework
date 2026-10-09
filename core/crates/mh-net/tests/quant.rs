//! The exe's network quantization (src/quant.rs: WritePackedVector, FRepMovement levels, FRotator compression) and
//! relevancy / priority (src/relevancy.rs: APawn::IsNetRelevantFor distance culling, AAdvancedCharacter::GetNetPriority,
//! the send order). Expected values follow the disassembled formulas; the .rdata literals are checked against
//! extract/native/rdata.tsv when the extract is on this machine (SKIP otherwise).

use std::path::Path;

use mh_net::quant::*;
use mh_net::relevancy::*;
use mordhau_core::ue::FVector;

fn v(x: f32, y: f32, z: f32) -> FVector {
    FVector { x, y, z }
}

/// FMath::RoundToInt's SSE form (x + x + 0.5, round to nearest even, >> 1) is floor(x + 0.5)
#[test]
fn round_to_int_is_floor_half_up() {
    for (x, r) in [(0.5f32, 1), (-0.5, 0), (2.5, 3), (-2.5, -2), (1.49, 1), (-1.51, -2), (0.0, 0), (123456.78, 123457)] {
        assert_eq!(round_to_int(x), r, "RoundToInt({x})");
    }
}

#[test]
fn ceil_log_two_values() {
    for (a, r) in [(0u32, 0u32), (1, 0), (2, 1), (3, 2), (4, 2), (5, 3), (24, 5), (30, 5), (131072, 17), (131073, 18)] {
        assert_eq!(ceil_log_two(a), r, "CeilLogTwo({a})");
    }
}

/// WritePackedVector: the per-component bit count from the largest magnitude, the bias, the wire size; the reader
/// gets the rounded value back (scaled by 0.01f / 0.1f)
#[test]
fn packed_vector_round_trip() {
    let p = write_packed_vector(FVector::ZERO, 100, 30);
    assert_eq!((p.bits, p.d, p.clamped), (0, [2, 2, 2], false));
    assert_eq!(p.wire_bits(), 5 + 3 * 2);
    let p = write_packed_vector(v(1.0, -1.0, 0.0), 1, 24);
    // max |int| 1 -> CeilLogTwo(2) = 1 -> Bits 0: components in [0, 4) with bias 2
    assert_eq!((p.bits, p.d), (0, [3, 1, 2]));
    let p = write_packed_vector(v(1234.5678, -98.7654, 98.15), 100, 30);
    assert_eq!(p.bits, 16);
    let r = read_packed_vector(&p);
    assert!((r.x - 1234.57).abs() < 2e-4 && (r.y + 98.77).abs() < 2e-4 && (r.z - 98.15).abs() < 2e-4, "{r:?}");
    // NaN components send a zero vector
    let r = read_packed_vector(&write_packed_vector(v(f32::NAN, 5.0, 5.0), 10, 24));
    assert_eq!(r, FVector::ZERO);
    // out of range: clamped (WritePackedVector returns false)
    let p = write_packed_vector(v(3.0e7, 0.0, 0.0), 100, 30);
    assert!(p.clamped);
}

/// the precision of each level: location of a pawn to 1/100 cm, an actor's to 1 cm; NetQuantize10 to 0.1
#[test]
fn quantization_levels() {
    let x = v(10.126, -3.333, 0.5);
    let (r, _) = quantize_vector(x, PAWN_REP_QUANTIZATION.location);
    assert!((r.x - 10.13).abs() < 1e-4 && (r.y + 3.33).abs() < 1e-4 && (r.z - 0.5).abs() < 1e-6, "{r:?}");
    let (r, _) = quantize_vector(x, ACTOR_REP_QUANTIZATION.location);
    assert_eq!(r, v(10.0, -3.0, 1.0)); // RoundToInt(0.5) = 1
    let r = net_quantize10(x);
    assert!((r.x - 10.1).abs() < 1e-5 && (r.y + 3.3).abs() < 1e-5 && (r.z - 0.5).abs() < 1e-6, "{r:?}");
    assert_eq!(PAWN_REP_QUANTIZATION.velocity, VectorQuantization::RoundWholeNumber);
    assert_eq!(PAWN_REP_QUANTIZATION.rotation, RotatorQuantization::ByteComponents);
}

/// FRotator::SerializeCompressed / SerializeCompressedShort: byte and short axes
#[test]
fn rotator_compression() {
    assert_eq!(compress_axis_to_byte(180.0), 128);
    assert_eq!(compress_axis_to_byte(359.9), 0); // RoundToInt(255.93) = 256 & 0xff
    assert_eq!(compress_axis_to_byte(-90.0), 192);
    assert_eq!(decompress_axis_from_byte(64), 90.0);
    assert_eq!(compress_axis_to_short(90.0), 16384);
    assert_eq!(decompress_axis_from_short(16384), 90.0);
    assert_eq!(rotator_wire_bits([0, 65, 0], RotatorQuantization::ByteComponents), 11);
    assert_eq!(rotator_wire_bits([1, 65, 0], RotatorQuantization::ShortComponents), 35);
}

/// AActor::IsWithinNetRelevancyDistance: strictly inside NetCullDistanceSquared (15000^2 for every Mordhau pawn)
#[test]
fn distance_culling() {
    assert_eq!(ACTOR_NET_CULL_DISTANCE_SQUARED, 15000.0 * 15000.0);
    let o = FVector::ZERO;
    assert!(is_within_net_relevancy_distance(v(14999.0, 0.0, 0.0), o, ACTOR_NET_CULL_DISTANCE_SQUARED));
    assert!(!is_within_net_relevancy_distance(v(15000.0, 0.0, 0.0), o, ACTOR_NET_CULL_DISTANCE_SQUARED));
    let q = PawnRelevancyQuery {
        always_relevant: false,
        viewer_is_controller: false,
        tied_to_viewer: false,
        hidden_without_collision: false,
        base_relevant: None,
        location: v(20000.0, 0.0, 0.0),
        net_cull_distance_squared: ACTOR_NET_CULL_DISTANCE_SQUARED,
    };
    assert!(!pawn_is_net_relevant_for(&q, o));
    assert!(pawn_is_net_relevant_for(&PawnRelevancyQuery { viewer_is_controller: true, ..q }, o));
    assert!(pawn_is_net_relevant_for(&PawnRelevancyQuery { location: v(100.0, 0.0, 0.0), ..q }, o));
    // a different duel room (SharesInstanceWith false) is never relevant
    assert!(!advanced_character_is_net_relevant_for(false, &PawnRelevancyQuery { location: v(100.0, 0.0, 0.0), ..q }, o));
}

/// AAdvancedCharacter::GetNetPriority: the view target x4; in front within 500 cm x2; behind 500..1500 cm x0.4; the
/// 45 degree cone; far x0.2; all times NetPriority 3
#[test]
fn pawn_priority() {
    let (o, fwd, t) = (FVector::ZERO, v(1.0, 0.0, 0.0), 0.5f32);
    let p = |l: FVector| advanced_character_get_net_priority(o, fwd, Some(l), false, t, PAWN_NET_PRIORITY);
    assert_eq!(advanced_character_get_net_priority(o, fwd, Some(v(400.0, 0.0, 0.0)), true, t, PAWN_NET_PRIORITY), t * 4.0 * 3.0);
    assert_eq!(p(v(400.0, 0.0, 0.0)), (t + t) * 3.0);
    assert_eq!(p(v(-600.0, 0.0, 0.0)), t * 0.4 * 3.0);
    assert_eq!(p(v(-400.0, 0.0, 0.0)), t * 3.0);
    assert_eq!(p(v(-2000.0, 0.0, 0.0)), t * 0.2 * 3.0);
    assert_eq!(p(v(2000.0, 0.0, 0.0)), t * 3.0); // 1500..3000 in the cone
    assert_eq!(p(v(1000.0, 2000.0, 0.0)), t * 0.4 * 3.0); // 1500..3000 outside the cone
    assert_eq!(p(v(5000.0, 0.0, 0.0)), t * 0.4 * 3.0); // 3000..6000 in the cone
    assert_eq!(p(v(7000.0, 0.0, 0.0)), t * 0.2 * 3.0);
    assert_eq!(advanced_character_get_net_priority(o, fwd, None, false, t, PAWN_NET_PRIORITY), t * 3.0);
}

/// the send loop: highest priority first, until the frame's byte budget is used
#[test]
fn send_order_by_priority_within_budget() {
    assert_eq!(send_order(&[1.0, 3.0, 2.0], &[10, 10, 10], 15.0), vec![1, 2]);
    assert_eq!(send_order(&[1.0, 3.0, 2.0], &[10, 10, 10], 1000.0), vec![1, 2, 0]);
    assert!(send_order(&[1.0], &[10], 0.0).is_empty());
}

/// the .rdata literals of quant.rs / relevancy.rs against extract/native/rdata.tsv (only the rows the extract has:
/// it lists literals Mordhau code references)
#[test]
fn rdata_literals() {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../extract/native/rdata.tsv");
    let Ok(t) = std::fs::read_to_string(&p) else { return eprintln!("SKIP: {} missing", p.display()) };
    let want: &[(u64, u32)] = &[
        (0x1443247b8, 250_000f32.to_bits()),
        (0x1443247bc, 2_250_000f32.to_bits()),
        (0x1443247c0, 9_000_000f32.to_bits()),
        (0x1443247c4, 36_000_000f32.to_bits()),
        (0x14433115c, 0x3f36_0b61),
        (0x144331160, 1.40625f32.to_bits()),
        (0x1443754f0, 0x4336_0b61),
        (0x143fe4dfc, 0x3dcc_cccd),
        (0x144014a94, 0x3c23_d70a),
    ];
    let mut seen = 0;
    for line in t.lines().skip(1) {
        let c: Vec<&str> = line.split('\t').collect();
        let Ok(va) = u64::from_str_radix(c[0], 16) else { continue };
        if let Some(&(_, bits)) = want.iter().find(|w| w.0 == va) {
            let raw = u32::from_str_radix(c[3], 16).unwrap().swap_bytes();
            assert_eq!(raw, bits, ".rdata {va:#x}");
            seen += 1;
        }
    }
    assert_eq!(seen, want.len(), "rows missing from rdata.tsv");
}

/// world_rep: changed placed-actor properties go out once per client; the native objectives at most every 0.1 s
/// (NetUpdateFrequency 10), the Blueprint actors the next frame (ForceNetUpdate); the initial state always goes out
#[test]
fn world_props_diff() {
    use mh_net::msg::Msg;
    use mh_net::world_rep::WorldRepServer;
    let mut s = WorldRepServer::default();
    let v = |door: i64, prog: i64| vec![(1u32, "DoorState".to_string(), door), (2u32, "ReplicatedProgress".to_string(), prog)];
    assert_eq!(s.diff(7, 1.0, &v(0, 0)).len(), 2, "initial replication");
    assert!(s.diff(7, 1.0, &v(0, 0)).is_empty());
    let m = s.diff(7, 1.02, &v(1, 100));
    assert_eq!(m, vec![Msg::WorldProp { actor: 1, var: "DoorState".into(), value: 1 }], "pushable held to its 10 Hz");
    let m = s.diff(7, 1.11, &v(1, 200));
    assert_eq!(m, vec![Msg::WorldProp { actor: 2, var: "ReplicatedProgress".into(), value: 200 }]);
    assert_eq!(s.diff(8, 1.11, &v(1, 200)).len(), 2, "another client gets its own initial state");
}

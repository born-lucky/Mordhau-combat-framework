//! The replicated stat byte (AAdvancedCharacter ReplicatedHealth +0x9d2, AMordhauCharacter ReplicatedStamina +0xe68):
//! the authority writes it when a stat changes with bReplicate, clients read it back in the property's OnRep.
//! Integer stat values (UStatComponent StatValue +0xb0, Min +0xbc, Max +0xc0). Port of godot/game/net/net_stat.gd.
//! Doubles as the GDScript reference computes them (the exe uses single precision; the >> 1 of a value within 0.5 of an
//! integer is the same).

use crate::consts::*;
use crate::ue::{clampf, cvtss2si};

const fn d(x: f32) -> f64 {
    x as f64
}

/// from UStatComponent::WriteReplicatedStat rva=0x1520b50:
///   fraction = (Value - Min) / (Max - Min) clamped to [0, 1]; a degenerate range (|Max - Min| <= 1e-8) gives 1 when
///   Value >= Max, else 0
///   byte = cvtss2si(fraction x 200 + 0.5) >> 1 (RoundToInt(fraction x 100)); 0 while Value != 0 -> 1
///   byte == the old byte -> byte | 0x80: the high bit flips on every write, so the property always changes and every
///   write reaches the clients' OnRep even when the percent is the same
pub fn write_replicated_stat(value: i64, mn: i64, mx: i64, old_byte: u8) -> u8 {
    write_replicated_stat_q(value, mn, mx, old_byte, |x| x)
}

/// the same with the machine's float model: `q` = identity (the GDScript reference, doubles) or binary32 rounding of
/// every operation (the exe: subss / divss / mulss / addss)
pub fn write_replicated_stat_q(value: i64, mn: i64, mx: i64, old_byte: u8, q: fn(f64) -> f64) -> u8 {
    let span = q(q(mx as f64) - q(mn as f64));
    let mut f = 0.0f64;
    if span.abs() > d(STAT_SMALL_NUMBER) {
        f = q(q(q(value as f64) - q(mn as f64)) / span);
    } else if value as f64 >= mx as f64 {
        f = d(STAT_UNIT_MAX);
    }
    f = clampf(f, 0.0, d(STAT_UNIT_MAX));
    let mut b = ((cvtss2si(q(q(f * d(STAT_UNIT_TO_BYTE_X2)) + d(STAT_ROUND_HALF))) >> 1) & 0xff) as u8;
    if b == 0 && value != 0 {
        b = 1;
    }
    if old_byte == b {
        b |= 0x80;
    }
    b
}

/// from UStatComponent::ReadReplicatedStat rva=0x1513260 (disassembled 0x141513260..0x1415132df):
///   f = clamp((Byte & 0x7f) x 0.01, 0, 1)
///   value = cvtss2si((Max - Min) x (f + f) + Min + (Min + 0.5)) >> 1
///   value != StatValue -> SetStatValue_Internal(value, bReplicate false) (vtable +0x408), which for health broadcasts
///   OnDied when it goes from > 0 to < 1 (UHealthStatComponent::SetStatValue_Internal rva=0x14d5cf0)
pub fn read_replicated_stat(byte: u8, mn: i64, mx: i64) -> i64 {
    read_replicated_stat_q(byte, mn, mx, |x| x)
}

/// the same with the machine's float model (see write_replicated_stat_q)
pub fn read_replicated_stat_q(byte: u8, mn: i64, mx: i64, q: fn(f64) -> f64) -> i64 {
    let f = clampf(q((byte & 0x7f) as f64 * d(STAT_BYTE_TO_UNIT)), 0.0, d(STAT_UNIT_MAX));
    let (mnf, mxf) = (q(mn as f64), q(mx as f64));
    cvtss2si(q(q(q(q(mxf - mnf) * q(f + f)) + mnf) + q(mnf + d(STAT_ROUND_HALF)))) >> 1
}

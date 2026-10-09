//! Network quantization of vectors and rotators, read off the exe (engine code statically linked, UE 4.26 layout):
//!
//! - FRepMovement (AActor::ReplicatedMovement, AActor +0x60): LinearVelocity +0x0, AngularVelocity +0xc, Location
//!   +0x18, Rotation +0x24, bits +0x30 (bSimulatedPhysicSleep, bRepPhysics), LocationQuantizationLevel +0x31,
//!   VelocityQuantizationLevel +0x32, RotationQuantizationLevel +0x33. Its constructor FRepMovement::FRepMovement
//!   rva=0x309c260 zeroes the three levels (`mov word [rcx+0x31], ax` / `mov byte [rcx+0x33], al`): RoundWholeNumber,
//!   RoundWholeNumber, ByteComponents. APawn::APawn rva=0x32ddce0 then writes LocationQuantizationLevel = 2
//!   (RoundTwoDecimals) through AActor::GetReplicatedMovement_Mutable 0x1dda400 (`lea rax, [rcx+0x60]`) at
//!   0x1432dde76 (`mov byte [rax+0x31], 2`). Nothing later changes them for Mordhau's pawns: no other write in
//!   ACharacter 0x2f27200, AAdvancedCharacter 0x144b0e0, AMordhauCharacter 0x1524d90 or AHorse 0x14e36f0 constructors,
//!   and BP_MordhauCharacter / BP_Horse carry no ReplicatedMovement override in their packages. So a pawn replicates
//!   Location to 1/100 cm, LinearVelocity to whole cm/s, Rotation as bytes.
//! - FRepMovement NetSerialize = UScriptStruct::TCppStructOps<FRepMovement>::NetSerialize rva=0x35d9d00: 2 bits
//!   (bSimulatedPhysicSleep, bRepPhysics), SerializeQuantizedVector(Location, LocationQuantizationLevel), Rotation
//!   (level 0 FRotator::SerializeCompressed rva=0x18bf2e0, 1 SerializeCompressedShort 0x18bf4e0),
//!   SerializeQuantizedVector(LinearVelocity, VelocityQuantizationLevel), AngularVelocity only with bRepPhysics.
//! - FRepMovement::SerializeQuantizedVector rva=0x35d9de0: level 0 -> WritePackedVector<1,24> 0x35d6c30, level 1 ->
//!   <10,27> 0x35d6f00, level 2 -> <100,30> 0x2f6c380; the read side (inlined) converts (D - Bias) to float and
//!   multiplies by 0.1f (.rdata 0x143fe4dfc) / 0.01f (0x144014a94); level 0 does not scale.
//! - WritePackedVector<Scale, MaxBits> (e.g. 0x2f6c380): Value * Scale (`mulss` by .rdata 100 / 10; none for 1), a NaN
//!   test per component (zero vector), clamp to [-1073741824, 1073741760] (.rdata 0x144971184 / 0x14497117c),
//!   FMath::RoundToInt (`addss x, x; addss 0.5; cvtss2si; sar 1`), Bits = clamp(CeilLogTwo(1 + max |int|), 1, MaxBits)
//!   - 1 (the bsr sequence at 0x142f6c4cf..0x142f6c50b), SerializeInt(Bits, MaxBits), Bias = 1 << (Bits + 1), Max =
//!   1 << (Bits + 2), each component D = int + Bias clamped into [0, Max) and SerializeInt(D, Max).
//! - ServerMove's FVector_NetQuantize100 / 10 / NetQuantize: SerializePackedVector<100,30> rva=0x2f6bf60 (read * 0.01f),
//!   <10,24> 0x2f6be20 (read * 0.1f), FVector_NetQuantize::NetSerialize 0x2f3d200 (WritePackedVector<1,20> 0x2f26af0).
//! - FRotator::SerializeCompressed rva=0x18bf2e0: each axis RoundToInt(Angle * 0.7111111f) & 0xFF (.rdata 0x14433115c
//!   = 0x3f360b61), a 1-bit "non-zero" flag then 8 bits; read Byte * 1.40625f (0x144331160). SerializeCompressedShort
//!   0x18bf4e0: RoundToInt(Angle * 182.04445f) & 0xFFFF (0x1443754f0 = 0x43360b61); read Short * 0.0054931640625f
//!   (0x144452430). Axis order Pitch, Yaw, Roll.

use mordhau_core::ue::FVector;
use serde::{Deserialize, Serialize};

/// EVectorQuantization
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VectorQuantization {
    RoundWholeNumber = 0,
    RoundOneDecimal = 1,
    RoundTwoDecimals = 2,
}

/// ERotatorQuantization
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RotatorQuantization {
    ByteComponents = 0,
    ShortComponents = 1,
}

/// FRepMovement's quantization levels of an actor
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RepQuantization {
    pub location: VectorQuantization,
    pub velocity: VectorQuantization,
    pub rotation: RotatorQuantization,
}

/// FRepMovement::FRepMovement rva=0x309c260 (every actor)
pub const ACTOR_REP_QUANTIZATION: RepQuantization = RepQuantization { location: VectorQuantization::RoundWholeNumber, velocity: VectorQuantization::RoundWholeNumber, rotation: RotatorQuantization::ByteComponents };

/// APawn::APawn rva=0x32ddce0 (0x1432dde76: LocationQuantizationLevel = RoundTwoDecimals): every Mordhau pawn
/// (characters, horses)
pub const PAWN_REP_QUANTIZATION: RepQuantization = RepQuantization { location: VectorQuantization::RoundTwoDecimals, velocity: VectorQuantization::RoundWholeNumber, rotation: RotatorQuantization::ByteComponents };

/// FMath::RoundToInt (SSE): cvtss2si(X + X + 0.5) >> 1 (round half up; cvtss2si rounds to nearest even, exactly as
/// the doubled value needs). Out of cvtss2si's range gives the integer indefinite 0x80000000.
pub fn round_to_int(x: f32) -> i32 {
    let t = x + x + 0.5;
    if !(-2147483648.0..2147483648.0).contains(&t) {
        return i32::MIN >> 1;
    }
    (t.round_ties_even() as i32) >> 1
}

/// one WritePackedVector: what goes on the wire
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackedVector {
    pub scale: u32,
    pub max_bits: u32,
    /// the per-component bit count less 2 (SerializeInt(Bits, MaxBits))
    pub bits: u32,
    /// the three biased components
    pub d: [u32; 3],
    /// the value did not fit (WritePackedVector's return value is false)
    pub clamped: bool,
}

impl PackedVector {
    /// bits on the wire: SerializeInt(Bits, MaxBits) takes CeilLogTwo(MaxBits) bits (FBitWriter::SerializeInt writes
    /// the bits a value below Max needs), then 3 x (Bits + 2)
    pub fn wire_bits(&self) -> u32 {
        ceil_log_two(self.max_bits) + 3 * (self.bits + 2)
    }
}

/// FMath::CeilLogTwo(Arg): 0 for 0 and 1, else the bit length of Arg - 1
pub fn ceil_log_two(arg: u32) -> u32 {
    if arg <= 1 {
        0
    } else {
        32 - (arg - 1).leading_zeros()
    }
}

/// WritePackedVector<Scale, MaxBits> (module docs)
pub fn write_packed_vector(v: FVector, scale: u32, max_bits: u32) -> PackedVector {
    let mut s = if scale != 1 { FVector { x: v.x * scale as f32, y: v.y * scale as f32, z: v.z * scale as f32 } } else { v };
    if s.x.is_nan() || s.y.is_nan() || s.z.is_nan() {
        s = FVector::ZERO;
    }
    // ClampVector: x >= Min ? min(x, Max) : Min (comiss / minss at 0x142f6c3fc..)
    let cl = |x: f32| if x >= -1073741824.0 { x.min(1073741760.0) } else { -1073741824.0 };
    let c = FVector { x: cl(s.x), y: cl(s.y), z: cl(s.z) };
    let mut clamped = c.x != s.x || c.y != s.y || c.z != s.z;
    let i = [round_to_int(c.x), round_to_int(c.y), round_to_int(c.z)];
    let m = i.iter().map(|x| x.unsigned_abs()).max().unwrap_or(0);
    let bits = ceil_log_two(m.wrapping_add(1)).clamp(1, max_bits) - 1;
    let bias = 1u32 << (bits + 1);
    let max = 1u32 << (bits + 2);
    let mut d = [0u32; 3];
    for k in 0..3 {
        let mut dk = (i[k] as u32).wrapping_add(bias);
        if dk >= max {
            clamped = true;
            dk = if (dk as i32) > 0 { max - 1 } else { 0 };
        }
        d[k] = dk;
    }
    PackedVector { scale, max_bits, bits, d, clamped }
}

/// the receiving side: (D - Bias) as float, times 0.1f / 0.01f for scale 10 / 100 (none for 1)
pub fn read_packed_vector(p: &PackedVector) -> FVector {
    let bias = 1i32 << (p.bits + 1);
    let f = |d: u32| (d as i32).wrapping_sub(bias) as f32;
    let k = match p.scale {
        1 => None,
        10 => Some(f32::from_bits(0x3dcc_cccd)),  // 0.1f, .rdata 0x143fe4dfc
        100 => Some(f32::from_bits(0x3c23_d70a)), // 0.01f, .rdata 0x144014a94
        s => Some(1.0 / s as f32),
    };
    let (x, y, z) = (f(p.d[0]), f(p.d[1]), f(p.d[2]));
    match k {
        None => FVector { x, y, z },
        Some(k) => FVector { x: x * k, y: y * k, z: z * k },
    }
}

/// FRepMovement::SerializeQuantizedVector rva=0x35d9de0: level -> (Scale, MaxBits)
pub fn level_packing(q: VectorQuantization) -> (u32, u32) {
    match q {
        VectorQuantization::RoundWholeNumber => (1, 24),
        VectorQuantization::RoundOneDecimal => (10, 27),
        VectorQuantization::RoundTwoDecimals => (100, 30),
    }
}

/// what the receiver of a SerializeQuantizedVector gets, and the bits it cost
pub fn quantize_vector(v: FVector, q: VectorQuantization) -> (FVector, u32) {
    let (s, b) = level_packing(q);
    let p = write_packed_vector(v, s, b);
    (read_packed_vector(&p), p.wire_bits())
}

/// FVector_NetQuantize100 (SerializePackedVector<100,30> rva=0x2f6bf60)
pub fn net_quantize100(v: FVector) -> FVector {
    read_packed_vector(&write_packed_vector(v, 100, 30))
}

/// FVector_NetQuantize10 (SerializePackedVector<10,24> rva=0x2f6be20)
pub fn net_quantize10(v: FVector) -> FVector {
    read_packed_vector(&write_packed_vector(v, 10, 24))
}

/// FVector_NetQuantize (FVector_NetQuantize::NetSerialize rva=0x2f3d200: WritePackedVector<1,20>)
pub fn net_quantize(v: FVector) -> FVector {
    read_packed_vector(&write_packed_vector(v, 1, 20))
}

/// FRotator::CompressAxisToByte as SerializeCompressed rva=0x18bf2e0 computes it
pub fn compress_axis_to_byte(angle: f32) -> u8 {
    (round_to_int(angle * f32::from_bits(0x3f36_0b61)) & 0xff) as u8
}
/// FRotator::DecompressAxisFromByte (SerializeCompressed's read: * 1.40625f, .rdata 0x144331160)
pub fn decompress_axis_from_byte(b: u8) -> f32 {
    b as f32 * 1.40625
}
/// FRotator::CompressAxisToShort as SerializeCompressedShort rva=0x18bf4e0 computes it
pub fn compress_axis_to_short(angle: f32) -> u16 {
    (round_to_int(angle * f32::from_bits(0x4336_0b61)) & 0xffff) as u16
}
/// FRotator::DecompressAxisFromShort (* 0.0054931640625f, .rdata 0x144452430)
pub fn decompress_axis_from_short(s: u16) -> f32 {
    s as f32 * f32::from_bits(0x3bb4_0000)
}

/// bits of a compressed rotator: a flag per axis, 8 (byte) or 16 (short) more for a non-zero one
pub fn rotator_wire_bits(c: [u16; 3], q: RotatorQuantization) -> u32 {
    let w = if q == RotatorQuantization::ByteComponents { 8 } else { 16 };
    c.iter().map(|&x| if x != 0 { 1 + w } else { 1 }).sum()
}

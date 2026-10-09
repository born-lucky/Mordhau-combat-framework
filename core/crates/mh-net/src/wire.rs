//! The engine's wire forms (rust-net r8, replacing the JSON payloads): FBitWriter / FBitReader bit packing and every
//! message's parameters serialized as the exe serializes the matching property / RPC parameter / struct. Read off the
//! disassembly:
//!   - FBitWriter::SerializeInt rva=0x19335d0: Value >= ValueMax is clamped to ValueMax - 1; then bits LSB first, mask 1,
//!     2, 4, ... while (NewValue + Mask) < ValueMax (0x141933790..0x1419337c5) - a variable bit count, not always
//!     CeilLogTwo(ValueMax). FBitWriter::WriteIntWrapped rva=0x1936170: the same loop without the clamp.
//!   - FBitWriter::SerializeIntPacked rva=0x1933a20: 7-bit groups, each byte = (group << 1) | (more groups follow)
//!     (0x141933a50..0x141933a82), written as 8 x count bits.
//!   - FBitWriter::WriteBit rva=0x1935e10 / SerializeBits rva=0x1932860: bits in byte order, LSB first (GShift table
//!     .rdata 0x1444ab778); a multi-byte value is its little-endian bytes (FArchive << on the bit archive).
//!   - FVector_NetQuantize* / FRepMovement / FRotator compression: quant.rs (WritePackedVector, SerializeCompressed /
//!     SerializeCompressedShort, FRotator::NetSerialize rva=0x18b55d0 = SerializeCompressedShort).
//!   - FCharacterNetworkMoveData::Serialize rva=0x2f866b0: TimeStamp (float), Acceleration SerializePackedVector<10,24>
//!     (0x2f6be20), Location FVector_NetQuantize100 (WritePackedVector<100,30> 0x2f6c380), ControlRotation
//!     FRotator::NetSerialize, CompressedMoveFlags SerializeOptionalValue<uint8>(default 0) (0x2f6bd60: a bit "differs
//!     from the default", then the byte); NewMove only: MovementBase / MovementBaseBoneName optional (a 0 bit each
//!     here), MovementMode SerializeOptionalValue<uint8>(default MOVE_Walking 1). FCharacterNetworkMoveDataContainer::
//!     Serialize rva=0x2f86950: NewMove, bIsDualMove bit (+ bDisableCombinedScopedMove bit + PendingMove), bHasOldMove
//!     bit (+ OldMove). FCharacterNetworkSerializationPackedBits::NetSerialize rva=0x2f7cd60: SerializeIntPacked(NumBits)
//!     + the bits (ServerMovePacked / ClientMoveResponsePacked: ShouldUsePackedMovementRPCs, vtable +0xa10 = the
//!     engine's 0x2f8ade0 for every Mordhau movement class).
//!   - FCharacterMoveResponseDataContainer::Serialize rva=0x2f862c0: bAckGoodMove bit, TimeStamp float; a correction:
//!     bHasBase, bHasRotation, bRootMotionMontageCorrection, bRootMotionSourceCorrection bits, NewLoc and NewVel
//!     FVector::NetSerialize rva=0x18b56c0 (three full floats each), NewBase / NewBaseBoneName optional, MovementMode
//!     SerializeOptionalValue<uint8>(default 1).
//!   - UPackageMap::StaticSerializeName rva=0x1ae01e0 for FName parameters.
//! UNCONFIRMED (listed in docs/RUST_NET.md section 10): the field handles (an index per message kind here; the engine
//! numbers RepLayout commands and the class's NetFields), object references (a pawn is its channel; other references
//! and the actor export carry the host's actor name as an FString where the engine sends NetGUIDs and an archetype
//! export), the spawn generation number (`gen`), the controller-property and world-actor handle numbering, and the
//! session / beacon control messages' parameters beyond the NMT names.

use crate::layout::PropValue;
use crate::movement::{MoveResponse, OldMoveData, RepMovement, ServerMoveData, ServerMovePacket};
use crate::msg::{Entity, Msg};
use crate::quant;
use mordhau_core::ue::FVector;

// ---- FBitWriter / FBitReader ----------------------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct BitWriter {
    pub buf: Vec<u8>,
    pub num: usize,
}

impl BitWriter {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn bytes(&self) -> usize {
        self.num.div_ceil(8)
    }
    /// FBitWriter::WriteBit rva=0x1935e10
    pub fn write_bit(&mut self, b: bool) {
        if self.num % 8 == 0 {
            self.buf.push(0);
        }
        if b {
            let i = self.num / 8;
            self.buf[i] |= 1 << (self.num % 8);
        }
        self.num += 1;
    }
    /// FBitWriter::SerializeBits rva=0x1932860: `n` bits of `data`, LSB first
    pub fn write_bits(&mut self, data: &[u8], n: usize) {
        for i in 0..n {
            self.write_bit(data[i / 8] >> (i % 8) & 1 != 0);
        }
    }
    /// FBitWriter::SerializeInt rva=0x19335d0 (module docs)
    pub fn serialize_int(&mut self, v: u32, max: u32) {
        let v = if v >= max { max - 1 } else { v };
        self.write_int_wrapped(v, max);
    }
    /// FBitWriter::WriteIntWrapped rva=0x1936170
    pub fn write_int_wrapped(&mut self, v: u32, max: u32) {
        let mut new = 0u32;
        let mut mask = 1u32;
        while mask != 0 && new.wrapping_add(mask) < max {
            let b = v & mask != 0;
            self.write_bit(b);
            if b {
                new += mask;
            }
            mask = mask.wrapping_mul(2);
        }
    }
    /// FBitWriter::SerializeIntPacked rva=0x1933a20
    pub fn serialize_int_packed(&mut self, mut v: u32) {
        loop {
            let more = v & !0x7f != 0;
            let byte = (((v & 0x7f) << 1) | more as u32) as u8;
            self.write_bits(&[byte], 8);
            v >>= 7;
            if !more {
                break;
            }
        }
    }
    pub fn u8(&mut self, v: u8) {
        self.write_bits(&[v], 8);
    }
    pub fn u16(&mut self, v: u16) {
        self.write_bits(&v.to_le_bytes(), 16);
    }
    pub fn u32(&mut self, v: u32) {
        self.write_bits(&v.to_le_bytes(), 32);
    }
    pub fn i32(&mut self, v: i32) {
        self.u32(v as u32);
    }
    pub fn u64(&mut self, v: u64) {
        self.write_bits(&v.to_le_bytes(), 64);
    }
    pub fn f32(&mut self, v: f32) {
        self.u32(v.to_bits());
    }
    pub fn f64(&mut self, v: f64) {
        self.u64(v.to_bits());
    }
    /// FArchive << FString: int32 SaveNum = Len + 1 (ANSI) or -(Len + 1) (UCS-2) when a character is not ANSI, the
    /// characters and the terminating NUL; an empty string is SaveNum 0
    pub fn fstring(&mut self, s: &str) {
        if s.is_empty() {
            self.i32(0);
            return;
        }
        if s.chars().all(|c| (c as u32) < 128) {
            self.i32(s.len() as i32 + 1);
            for b in s.bytes() {
                self.u8(b);
            }
            self.u8(0);
        } else {
            let u: Vec<u16> = s.encode_utf16().collect();
            self.i32(-(u.len() as i32 + 1));
            for c in u {
                self.u16(c);
            }
            self.u16(0);
        }
    }
    /// UPackageMap::StaticSerializeName rva=0x1ae01e0: bHardcoded bit; hardcoded -> SerializeIntPacked(EName); else the
    /// FString and the int32 Number. NAME_None is hardcoded index 0.
    pub fn name(&mut self, s: &str) {
        if s.is_empty() || s == "None" {
            self.write_bit(true);
            self.serialize_int_packed(0);
            return;
        }
        self.write_bit(false);
        self.fstring(s);
        self.i32(0);
    }
    /// WritePackedVector<Scale, MaxBits> (quant.rs: the same arithmetic) onto the archive: SerializeInt(Bits, MaxBits)
    /// then the three components with SerializeInt(D, Max)
    pub fn packed_vector(&mut self, v: FVector, scale: u32, max_bits: u32) {
        let p = quant::write_packed_vector(v, scale, max_bits);
        self.serialize_int(p.bits, max_bits);
        let max = 1u32 << (p.bits + 2);
        for d in p.d {
            self.serialize_int(d, max);
        }
    }
    /// FRotator::SerializeCompressed (bytes) / SerializeCompressedShort: per axis a "non-zero" bit, then 8 / 16 bits
    pub fn rotator(&mut self, pitch: u16, yaw: u16, roll: u16, short: bool) {
        for a in [pitch, yaw, roll] {
            self.write_bit(a != 0);
            if a != 0 {
                if short {
                    self.u16(a);
                } else {
                    self.u8(a as u8);
                }
            }
        }
    }
    pub fn append(&mut self, o: &BitWriter) {
        self.write_bits(&o.buf, o.num);
    }
}

#[derive(Clone, Debug)]
pub struct BitReader<'a> {
    pub buf: &'a [u8],
    pub pos: usize,
    pub num: usize,
    pub overflow: bool,
}

impl<'a> BitReader<'a> {
    pub fn new(buf: &'a [u8], num: usize) -> Self {
        BitReader { buf, pos: 0, num, overflow: false }
    }
    pub fn left(&self) -> usize {
        self.num.saturating_sub(self.pos)
    }
    pub fn read_bit(&mut self) -> bool {
        if self.pos >= self.num {
            self.overflow = true;
            return false;
        }
        let b = self.buf[self.pos / 8] >> (self.pos % 8) & 1 != 0;
        self.pos += 1;
        b
    }
    pub fn read_bits(&mut self, n: usize) -> Vec<u8> {
        let mut out = vec![0u8; n.div_ceil(8)];
        for i in 0..n {
            if self.read_bit() {
                out[i / 8] |= 1 << (i % 8);
            }
        }
        out
    }
    /// FBitReader::SerializeInt (the writer's loop)
    pub fn serialize_int(&mut self, max: u32) -> u32 {
        let mut v = 0u32;
        let mut mask = 1u32;
        while mask != 0 && v.wrapping_add(mask) < max {
            if self.read_bit() {
                v |= mask;
            }
            mask = mask.wrapping_mul(2);
        }
        v
    }
    /// FBitReader::SerializeIntPacked rva=0x1933900
    pub fn serialize_int_packed(&mut self) -> u32 {
        let mut v = 0u32;
        let mut shift = 0;
        loop {
            let b = self.read_bits(8)[0];
            if shift < 32 {
                v |= ((b >> 1) as u32) << shift;
            }
            shift += 7;
            if b & 1 == 0 || self.overflow || shift > 35 {
                break;
            }
        }
        v
    }
    pub fn u8(&mut self) -> u8 {
        self.read_bits(8)[0]
    }
    pub fn u16(&mut self) -> u16 {
        let b = self.read_bits(16);
        u16::from_le_bytes([b[0], b[1]])
    }
    pub fn u32(&mut self) -> u32 {
        let b = self.read_bits(32);
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    }
    pub fn i32(&mut self) -> i32 {
        self.u32() as i32
    }
    pub fn u64(&mut self) -> u64 {
        let b = self.read_bits(64);
        u64::from_le_bytes(b.try_into().unwrap())
    }
    pub fn f32(&mut self) -> f32 {
        f32::from_bits(self.u32())
    }
    pub fn f64(&mut self) -> f64 {
        f64::from_bits(self.u64())
    }
    pub fn fstring(&mut self) -> String {
        let n = self.i32();
        if n == 0 {
            return String::new();
        }
        if n > 0 {
            let n = (n as usize).min(1 << 16);
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                v.push(self.u8());
            }
            v.pop();
            String::from_utf8_lossy(&v).into_owned()
        } else {
            let n = ((-n) as usize).min(1 << 16);
            let mut v = Vec::with_capacity(n);
            for _ in 0..n {
                v.push(self.u16());
            }
            v.pop();
            String::from_utf16_lossy(&v)
        }
    }
    pub fn name(&mut self) -> String {
        if self.read_bit() {
            let _ = self.serialize_int_packed();
            return String::new();
        }
        let s = self.fstring();
        let _ = self.i32();
        s
    }
    /// an FName as StaticSerializeName writes it: the hardcoded index, or the string (and its number)
    pub fn name_or_index(&mut self) -> Result<u32, String> {
        if self.read_bit() {
            return Ok(self.serialize_int_packed());
        }
        let s = self.fstring();
        let _ = self.i32();
        Err(s)
    }
    pub fn packed_vector(&mut self, scale: u32, max_bits: u32) -> FVector {
        let bits = self.serialize_int(max_bits);
        let max = 1u32 << (bits + 2);
        let d = [self.serialize_int(max), self.serialize_int(max), self.serialize_int(max)];
        quant::read_packed_vector(&quant::PackedVector { scale, max_bits, bits, d, clamped: false })
    }
    pub fn rotator(&mut self, short: bool) -> [u16; 3] {
        let mut out = [0u16; 3];
        for a in out.iter_mut() {
            if self.read_bit() {
                *a = if short { self.u16() } else { self.u8() as u16 };
            }
        }
        out
    }
}

// ---- message kinds and parameters ------------------------------------------------------------------------------------

/// the handle of a message kind (UNCONFIRMED numbering: module docs)
pub fn kind_of(m: &Msg) -> u32 {
    match m {
        Msg::ServerAssignNetMotion { .. } => 1,
        Msg::ServerDropParry { .. } => 2,
        Msg::ServerSuggestHitDetection { .. } => 3,
        Msg::ServerRequestDodge { .. } => 4,
        Msg::ClientSetNetMotion { .. } => 5,
        Msg::Prop { .. } => 6,
        Msg::PcProp { .. } => 7,
        Msg::Spawn { .. } => 8,
        Msg::Destroy { .. } => 9,
        Msg::BeaconConnect { .. } => 10,
        Msg::BeaconAccept => 11,
        Msg::BeaconRefused => 12,
        Msg::ServerPing => 13,
        Msg::ClientPong => 14,
        Msg::ServerReserveSlots { .. } => 15,
        Msg::ClientNotifyReservationStatus { .. } => 16,
        Msg::ServerMove { .. } => 17,
        Msg::RepMovement { .. } => 18,
        Msg::MovementSpawn { .. } => 19,
        Msg::MoveResponse { .. } => 20,
        Msg::ServerSetClimbLocation { .. } => 21,
        Msg::ServerRequestRearing { .. } => 22,
        Msg::MovementDestroy { .. } => 23,
        Msg::MovementPossess { .. } => 24,
        Msg::RepAttachment { .. } => 25,
        Msg::WorldProp { .. } => 26,
        Msg::Join { .. } => 27,
        Msg::JoinResult { .. } => 28,
        Msg::FrameDone { .. } => 29,
        Msg::FrameEnd { .. } => 30,
    }
}
pub const MAX_KIND: u32 = 30;

/// the actor a message is about (its channel); "" = the connection's control channel
pub fn subject(m: &Msg) -> String {
    match m {
        Msg::Spawn { name, .. } | Msg::Destroy { name } => name.clone(),
        Msg::WorldProp { actor, .. } => format!("#world{actor}"),
        Msg::PcProp { .. } => "#pc".into(),
        _ => m.who().to_string(),
    }
}

/// the AMordhauCharacter replicated properties (layout.rs CHARACTER_PROPS order) and the controller / world-actor
/// properties: handle numbering UNCONFIRMED
const PROPS: &[&str] = &[
    crate::enums::PROP_REPLICATED_NET_MOTION,
    crate::enums::PROP_REPLICATED_HEALTH,
    crate::enums::PROP_REPLICATED_STAMINA,
    crate::enums::PROP_REPLICATED_CHARACTER_FLAGS,
    crate::enums::PROP_REPLICATED_LOOK_UP_VALUE,
    crate::enums::PROP_REPLICATED_TEAM,
    crate::enums::PROP_NET_BLOCK,
    crate::enums::PROP_REPLICATED_KNOCKBACK,
    crate::enums::PROP_REPLICATED_DODGE,
];
/// world_rep.rs's variables and their engine widths (bool 1 bit, byte 8, the pushable's uint16 16)
const WORLD_VARS: &[(&str, usize)] = &[("DoorState", 8), ("ReplicatedHealth", 8), ("Regenerating", 1), ("LadderState", 8), ("Value", 1), ("ReplicatedProgress", 16), ("ReplicatedCaptureProgress", 8), ("OwningTeam", 8), ("CapturingTeam", 8)];

fn vec3(a: [f32; 3]) -> FVector {
    FVector { x: a[0], y: a[1], z: a[2] }
}

fn net_motion(w: &mut BitWriter, nm: &[u8; 6]) {
    // FNetMotion (sizeof 6: Id, MotionType, MotionParam0..2, MotionDynamicParam, all uint8: generated header
    // extract/match/gen FNetMotion.h) - each uint8 property 8 bits
    for b in nm {
        w.u8(*b);
    }
}
fn read_net_motion(r: &mut BitReader) -> [u8; 6] {
    let mut nm = [0u8; 6];
    for b in nm.iter_mut() {
        *b = r.u8();
    }
    nm
}

/// FCharacterNetworkMoveData::Serialize rva=0x2f866b0 (module docs)
fn move_data(w: &mut BitWriter, ts: f32, accel: [f32; 3], loc: [f32; 3], yaw: u16, flags: u8, mode: Option<u8>) {
    w.f32(ts);
    w.packed_vector(vec3(accel), 10, 24);
    w.packed_vector(vec3(loc), 100, 30);
    w.rotator(0, yaw, 0, true);
    w.write_bit(flags != 0);
    if flags != 0 {
        w.u8(flags);
    }
    if let Some(mode) = mode {
        w.write_bit(false); // MovementBase: none (static level collision)
        w.write_bit(false); // MovementBaseBoneName: NAME_None
        w.write_bit(mode != 1);
        if mode != 1 {
            w.u8(mode);
        }
    }
}
fn read_move_data(r: &mut BitReader, new_move: bool) -> (f32, [f32; 3], [f32; 3], u16, u8, u8) {
    let ts = r.f32();
    let a = r.packed_vector(10, 24);
    let l = r.packed_vector(100, 30);
    let rot = r.rotator(true);
    let flags = if r.read_bit() { r.u8() } else { 0 };
    let mut mode = 1;
    if new_move {
        if r.read_bit() {
            let _ = r.fstring(); // a movement base reference (never sent here)
        }
        if r.read_bit() {
            let _ = r.name();
        }
        if r.read_bit() {
            mode = r.u8();
        }
    }
    (ts, [a.x, a.y, a.z], [l.x, l.y, l.z], rot[1], flags, mode)
}

fn rep_movement(w: &mut BitWriter, r: &RepMovement) {
    // FRepMovement NetSerialize rva=0x35d9d00: bSimulatedPhysicSleep, bRepPhysics, Location (a pawn's level 2: <100,30>),
    // Rotation (level 0: bytes), LinearVelocity (level 0: <1,24>); then ACharacter ReplicatedMovementMode (uint8)
    w.write_bit(false);
    w.write_bit(false);
    w.packed_vector(vec3(r.location), 100, 30);
    w.rotator(0, r.yaw_byte as u16, 0, false);
    w.packed_vector(vec3(r.velocity), 1, 24);
    w.u8(r.mode);
}
fn read_rep_movement(rd: &mut BitReader) -> RepMovement {
    let _sleep = rd.read_bit();
    let _phys = rd.read_bit();
    let l = rd.packed_vector(100, 30);
    let rot = rd.rotator(false);
    let v = rd.packed_vector(1, 24);
    let mode = rd.u8();
    RepMovement { location: [l.x, l.y, l.z], velocity: [v.x, v.y, v.z], yaw_byte: rot[1] as u8, mode }
}

/// a message's parameters (the kind handle and the subject are written by the caller)
pub fn write_params(w: &mut BitWriter, m: &Msg) {
    match m {
        Msg::ServerAssignNetMotion { nm, last, .. } => {
            net_motion(w, nm);
            w.u8(*last);
        }
        Msg::ServerDropParry { id, .. } => w.u8(*id),
        Msg::ServerSuggestHitDetection { other, bone, .. } => {
            w.fstring(other); // AMordhauCharacter* OtherCharacter (UNCONFIRMED: a NetGUID in the engine)
            w.name(bone);
        }
        Msg::ServerRequestDodge { yaw, .. } => w.u8(*yaw),
        Msg::ClientSetNetMotion { nm, t, .. } => {
            net_motion(w, nm);
            w.f32(*t as f32); // ServerStartTime (float)
        }
        Msg::Prop { prop, v, .. } => {
            let h = PROPS.iter().position(|p| p == prop).unwrap_or(PROPS.len()) as u32;
            w.serialize_int_packed(h + 1);
            if h as usize == PROPS.len() {
                w.fstring(prop);
            }
            // the property's type follows from its handle (FNetMotion / FNetBlock structs, else a uint8)
            match v {
                PropValue::NetMotion(nm) => net_motion(w, nm),
                PropValue::Byte(b) => w.u8(*b),
                PropValue::NetBlock(nb) => {
                    // FNetBlock: BlockedReason, Flags, BlockedMove, Surface (uint8), BlockingActor (weak object
                    // reference: the actor name here, UNCONFIRMED), Version (uint8)
                    for b in [nb.reason, nb.flags, nb.mv, nb.surface] {
                        w.u8(b);
                    }
                    w.fstring(&nb.actor);
                    w.u8(nb.version);
                }
            }
        }
        Msg::PcProp { prop, v } => {
            // BP_DuelPlayerController ReplicatedRoomGame (ST_DuelRoomGame: Team1Wins / Team2Wins int, RoundInfo
            // {Stage byte, Winner byte, StartTime float}; widths UNCONFIRMED) - any other controller property as text
            w.fstring(prop);
            if prop == crate::duel_match::ROOM_GAME && v.get("round").is_some() {
                w.write_bit(true);
                w.i32(v["team1_wins"].as_i64().unwrap_or(0) as i32);
                w.i32(v["team2_wins"].as_i64().unwrap_or(0) as i32);
                w.u8(v["round"]["stage"].as_i64().unwrap_or(0) as u8);
                w.u8(v["round"]["winner"].as_i64().unwrap_or(0) as u8);
                w.f32(v["round"]["start_time"].as_f64().unwrap_or(0.0) as f32);
            } else {
                w.write_bit(false);
                w.fstring(&v.to_string());
            }
        }
        Msg::Spawn { owner, weapon, left, team, .. } => {
            // the actor export (UNCONFIRMED form: the engine's SerializeNewActor sends NetGUIDs and the archetype):
            // owning connection, RightHandEquipment / LeftHandEquipment paths, ReplicatedTeam
            w.serialize_int_packed(*owner);
            w.fstring(weapon);
            w.fstring(left);
            w.u8(*team);
        }
        Msg::Destroy { .. } => {}
        Msg::BeaconConnect { id } => w.serialize_int_packed(*id),
        Msg::BeaconAccept | Msg::BeaconRefused | Msg::ServerPing | Msg::ClientPong => {}
        Msg::ServerReserveSlots { entities } => {
            w.i32(entities.len() as i32);
            for e in entities {
                w.fstring(&e.id);
                w.i32(e.r#type);
            }
        }
        Msg::ClientNotifyReservationStatus { open, status } => {
            w.i32(*open);
            w.u8(*status);
        }
        Msg::ServerMove { gen, data, .. } => {
            w.serialize_int_packed(*gen);
            // ServerMovePacked(FCharacterServerMovePackedBits): SerializeIntPacked(NumBits) + the container
            let mut c = BitWriter::new();
            let n = &data.new;
            move_data(&mut c, n.time_stamp, n.accel, n.location, n.yaw, n.flags, Some(n.mode));
            c.write_bit(data.pending.is_some());
            if let Some(p) = &data.pending {
                c.write_bit(false); // bDisableCombinedScopedMove
                move_data(&mut c, p.time_stamp, p.accel, p.location, p.yaw, p.flags, Some(p.mode));
            }
            c.write_bit(data.old.is_some());
            if let Some(o) = &data.old {
                move_data(&mut c, o.time_stamp, o.accel, o.location, o.yaw, o.flags, None);
            }
            w.serialize_int_packed(c.num as u32);
            w.append(&c);
        }
        Msg::RepMovement { r, .. } => rep_movement(w, r),
        Msg::MovementSpawn { gen, r, kind, .. } => {
            w.serialize_int_packed(*gen);
            w.u8(*kind);
            rep_movement(w, r);
        }
        Msg::MoveResponse { r, .. } => {
            // ClientMoveResponsePacked(FCharacterMoveResponsePackedBits)
            let mut c = BitWriter::new();
            match r {
                MoveResponse::AckGoodMove { time_stamp } => {
                    c.write_bit(true);
                    c.f32(*time_stamp);
                }
                MoveResponse::AdjustPosition { time_stamp, location, velocity, mode } => {
                    c.write_bit(false);
                    c.f32(*time_stamp);
                    for _ in 0..4 {
                        c.write_bit(false); // bHasBase, bHasRotation, root motion montage / source corrections
                    }
                    for x in location.iter().chain(velocity.iter()) {
                        c.f32(*x);
                    }
                    c.write_bit(false); // NewBase
                    c.write_bit(false); // NewBaseBoneName
                    c.write_bit(*mode != 1);
                    if *mode != 1 {
                        c.u8(*mode);
                    }
                }
            }
            w.serialize_int_packed(c.num as u32);
            w.append(&c);
        }
        Msg::ServerSetClimbLocation { gen, target, .. } => {
            w.serialize_int_packed(*gen);
            w.packed_vector(vec3(*target), 1, 20); // FVector_NetQuantize (WritePackedVector<1,20> 0x2f26af0)
        }
        Msg::ServerRequestRearing { gen, .. } => w.serialize_int_packed(*gen),
        Msg::MovementDestroy { .. } => {}
        Msg::MovementPossess { gen, owned, .. } => {
            w.serialize_int_packed(*gen);
            w.write_bit(*owned);
        }
        Msg::RepAttachment { parent, offset, yaw, .. } => {
            // FRepAttachment: AttachParent (reference: the name here, UNCONFIRMED), LocationOffset NetQuantize100,
            // RelativeScale3D NetQuantize100, RotationOffset (FRotator::NetSerialize: compressed shorts), AttachSocket
            // (FName), AttachComponent (reference)
            w.write_bit(parent.is_some());
            if let Some(p) = parent {
                w.fstring(p);
            }
            w.packed_vector(vec3(*offset), 100, 30);
            w.packed_vector(FVector { x: 1.0, y: 1.0, z: 1.0 }, 100, 30);
            w.rotator(0, *yaw, 0, true);
            w.name("");
        }
        Msg::WorldProp { var, value, .. } => {
            let h = WORLD_VARS.iter().position(|(n, _)| n == var);
            w.serialize_int_packed(h.map(|i| i as u32 + 1).unwrap_or(0));
            match h {
                Some(i) => match WORLD_VARS[i].1 {
                    1 => w.write_bit(*value != 0),
                    8 => w.u8(*value as u8),
                    _ => w.u16(*value as u16),
                },
                None => {
                    w.fstring(var);
                    w.u64(*value as u64);
                }
            }
        }
        Msg::Join { name, options } => {
            // NMT_Login (RequestURL) - the player name as its own string here (UNCONFIRMED: the engine reads ?Name=)
            w.fstring(name);
            w.fstring(options);
        }
        Msg::JoinResult { error, player_id, tick_n, now } => {
            // NMT_Welcome / NMT_Failure (error text); PlayerId and the server clock (UNCONFIRMED: the engine
            // replicates them through the PlayerState / GameState)
            w.fstring(error);
            w.i32(*player_id);
            w.u64(*tick_n);
            w.f64(*now);
        }
        Msg::FrameDone { n } => w.u64(*n),
        Msg::FrameEnd { n, quit } => {
            w.u64(*n);
            w.write_bit(*quit);
        }
    }
}

/// the inverse of write_params; `who` = the subject the channel names
pub fn read_params(r: &mut BitReader, kind: u32, who: &str) -> Option<Msg> {
    let who = who.to_string();
    let m = match kind {
        1 => Msg::ServerAssignNetMotion { who, nm: read_net_motion(r), last: r.u8() },
        2 => Msg::ServerDropParry { who, id: r.u8() },
        3 => Msg::ServerSuggestHitDetection { who, other: r.fstring(), bone: r.name() },
        4 => Msg::ServerRequestDodge { who, yaw: r.u8() },
        5 => Msg::ClientSetNetMotion { who, nm: read_net_motion(r), t: r.f32() as f64 },
        6 => {
            let h = r.serialize_int_packed().checked_sub(1)? as usize;
            let prop = if h < PROPS.len() { PROPS[h].to_string() } else { r.fstring() };
            let v = if prop == crate::enums::PROP_REPLICATED_NET_MOTION {
                PropValue::NetMotion(read_net_motion(r))
            } else if prop == crate::enums::PROP_NET_BLOCK {
                let b = [r.u8(), r.u8(), r.u8(), r.u8()];
                let actor = r.fstring();
                PropValue::NetBlock(crate::rep::NetBlock { reason: b[0], flags: b[1], mv: b[2], surface: b[3], actor, version: r.u8() })
            } else {
                PropValue::Byte(r.u8())
            };
            Msg::Prop { who, prop, v }
        }
        7 => {
            let prop = r.fstring();
            let v = if r.read_bit() {
                let (t1, t2) = (r.i32(), r.i32());
                let (stage, winner, st) = (r.u8(), r.u8(), r.f32());
                serde_json::json!({"team1_wins": t1, "team2_wins": t2, "round": {"stage": stage, "winner": winner, "start_time": st as f64}})
            } else {
                serde_json::from_str(&r.fstring()).ok()?
            };
            Msg::PcProp { prop, v }
        }
        8 => Msg::Spawn { name: who, owner: r.serialize_int_packed(), weapon: r.fstring(), left: r.fstring(), team: r.u8() },
        9 => Msg::Destroy { name: who },
        10 => Msg::BeaconConnect { id: r.serialize_int_packed() },
        11 => Msg::BeaconAccept,
        12 => Msg::BeaconRefused,
        13 => Msg::ServerPing,
        14 => Msg::ClientPong,
        15 => {
            let n = r.i32().clamp(0, 4096);
            let mut entities = vec![];
            for _ in 0..n {
                entities.push(Entity { id: r.fstring(), r#type: r.i32() });
            }
            Msg::ServerReserveSlots { entities }
        }
        16 => Msg::ClientNotifyReservationStatus { open: r.i32(), status: r.u8() },
        17 => {
            let gen = r.serialize_int_packed();
            let nbits = r.serialize_int_packed() as usize;
            let start = r.pos;
            let (ts, a, l, yaw, flags, mode) = read_move_data(r, true);
            let new = ServerMoveData { time_stamp: ts, accel: a, location: l, flags, yaw, mode };
            let pending = if r.read_bit() {
                let _ = r.read_bit();
                let (ts, a, l, yaw, flags, mode) = read_move_data(r, true);
                Some(ServerMoveData { time_stamp: ts, accel: a, location: l, flags, yaw, mode })
            } else {
                None
            };
            let old = if r.read_bit() {
                let (ts, a, l, yaw, flags, _) = read_move_data(r, false);
                Some(OldMoveData { time_stamp: ts, accel: a, flags, location: l, yaw })
            } else {
                None
            };
            if r.pos - start != nbits {
                return None;
            }
            Msg::ServerMove { who, gen, data: ServerMovePacket { old, pending, new } }
        }
        18 => Msg::RepMovement { who, r: read_rep_movement(r) },
        19 => {
            let gen = r.serialize_int_packed();
            let kind = r.u8();
            Msg::MovementSpawn { who, gen, kind, r: read_rep_movement(r) }
        }
        20 => {
            let _nbits = r.serialize_int_packed();
            let ack = r.read_bit();
            let ts = r.f32();
            let resp = if ack {
                MoveResponse::AckGoodMove { time_stamp: ts }
            } else {
                for _ in 0..4 {
                    r.read_bit();
                }
                let v: Vec<f32> = (0..6).map(|_| r.f32()).collect();
                r.read_bit();
                r.read_bit();
                let mode = if r.read_bit() { r.u8() } else { 1 };
                MoveResponse::AdjustPosition { time_stamp: ts, location: [v[0], v[1], v[2]], velocity: [v[3], v[4], v[5]], mode }
            };
            Msg::MoveResponse { who, r: resp }
        }
        21 => {
            let gen = r.serialize_int_packed();
            let t = r.packed_vector(1, 20);
            Msg::ServerSetClimbLocation { who, gen, target: [t.x, t.y, t.z] }
        }
        22 => Msg::ServerRequestRearing { who, gen: r.serialize_int_packed() },
        23 => Msg::MovementDestroy { who },
        24 => Msg::MovementPossess { who, gen: r.serialize_int_packed(), owned: r.read_bit() },
        25 => {
            let parent = if r.read_bit() { Some(r.fstring()) } else { None };
            let o = r.packed_vector(100, 30);
            let _scale = r.packed_vector(100, 30);
            let rot = r.rotator(true);
            let _ = r.name();
            Msg::RepAttachment { who, parent, offset: [o.x, o.y, o.z], yaw: rot[1] }
        }
        26 => {
            let actor = who.strip_prefix("#world")?.parse().ok()?;
            let h = r.serialize_int_packed();
            let (var, value) = if h == 0 {
                (r.fstring(), r.u64() as i64)
            } else {
                let (n, bits) = *WORLD_VARS.get(h as usize - 1)?;
                let v = match bits {
                    1 => r.read_bit() as i64,
                    8 => r.u8() as i64,
                    _ => r.u16() as i64,
                };
                (n.to_string(), v)
            };
            Msg::WorldProp { actor, var, value }
        }
        27 => Msg::Join { name: r.fstring(), options: r.fstring() },
        28 => Msg::JoinResult { error: r.fstring(), player_id: r.i32(), tick_n: r.u64(), now: r.f64() },
        29 => Msg::FrameDone { n: r.u64() },
        30 => Msg::FrameEnd { n: r.u64(), quit: r.read_bit() },
        _ => return None,
    };
    if r.overflow {
        return None;
    }
    Some(m)
}

/// A message on its own (no connection: the in-process Loopback): subject as an FString, kind, parameters
pub fn encode_standalone(m: &Msg) -> Vec<u8> {
    let mut w = BitWriter::new();
    w.fstring(&subject(m));
    w.serialize_int(kind_of(m), MAX_KIND + 1);
    write_params(&mut w, m);
    let mut out = (w.num as u32).to_le_bytes().to_vec();
    out.extend_from_slice(&w.buf);
    out
}

pub fn decode_standalone(b: &[u8]) -> Option<Msg> {
    if b.len() < 4 {
        return None;
    }
    let n = u32::from_le_bytes(b[..4].try_into().ok()?) as usize;
    let mut r = BitReader::new(&b[4..], n.min((b.len() - 4) * 8));
    let who = r.fstring();
    let kind = r.serialize_int(MAX_KIND + 1);
    read_params(&mut r, kind, &who)
}

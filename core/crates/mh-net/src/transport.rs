//! The wire: an engine-agnostic transport trait the rules talk to, an in-memory loopback (tests, single-process
//! sessions) and a UDP transport (std::net) for real play. Not game rules: stand-ins for UE's net driver
//! (UIpNetDriver / UNetConnection, engine code). Delivery is reliable and ordered on both, as ServerAssignNetMotion /
//! ServerDropParry / ClientSetNetMotion are (enums.rs: FUNC_NetReliable). Endpoint 0 is the server (or beacon host),
//! 1.. the clients. Port of godot/game/net/net_loopback.gd and net_enet_transport.gd (ENet -> UDP).

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io;
use std::net::{SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use crate::msg::Msg;

pub trait Transport {
    /// queue `msg` from endpoint `from` to endpoint `to`
    fn send(&mut self, to: u32, msg: &Msg, from: u32);
    /// every message for `at` that is due by now, in send order: (from, msg)
    fn receive(&mut self, at: u32) -> Vec<(u32, Msg)>;
    /// the lockstep frame ends
    fn advance(&mut self);
    fn poll(&mut self) {}
    fn frame(&self) -> u64;
    /// (messages sent, bytes sent)
    fn stats(&self) -> (u64, u64);
}

/// An in-process transport: reliable, ordered, with a fixed latency in frames. Every message goes through
/// Msg::encode / decode, so what the rules send is exactly what a socket transport carries.
#[derive(Default)]
pub struct Loopback {
    pub latency_frames: u64,
    pub frame: u64,
    queues: HashMap<u32, VecDeque<(u64, u32, Vec<u8>)>>,
    pub sent: u64,
    pub bytes_sent: u64,
}

impl Loopback {
    pub fn new(latency_frames: u64) -> Self {
        Loopback { latency_frames, ..Default::default() }
    }
}

impl Transport for Loopback {
    fn send(&mut self, to: u32, msg: &Msg, from: u32) {
        let b = msg.encode();
        self.sent += 1;
        self.bytes_sent += b.len() as u64;
        self.queues.entry(to).or_default().push_back((self.frame + self.latency_frames, from, b));
    }

    fn receive(&mut self, at: u32) -> Vec<(u32, Msg)> {
        let mut out = Vec::new();
        let frame = self.frame;
        if let Some(q) = self.queues.get_mut(&at) {
            while q.front().is_some_and(|e| e.0 <= frame) {
                let (_, from, b) = q.pop_front().unwrap();
                if let Some(m) = Msg::decode(&b) {
                    out.push((from, m));
                }
            }
        }
        out
    }

    fn advance(&mut self) {
        self.frame += 1;
    }

    fn frame(&self) -> u64 {
        self.frame
    }

    fn stats(&self) -> (u64, u64) {
        (self.sent, self.bytes_sent)
    }
}

/// A loopback with a base latency plus a per-message random extra delay of 0..=jitter frames (a fixed-seed xorshift),
/// so later messages can overtake earlier ones, as unreliable datagrams do. For the movement tests (ServerMove is an
/// unreliable RPC in the engine); not for the reliable combat RPCs.
pub struct JitterLoopback {
    pub latency_frames: u64,
    pub jitter_frames: u64,
    /// probability that a message is lost (unreliable datagrams)
    pub loss: f64,
    pub lost: u64,
    pub frame: u64,
    seed: u64,
    queues: HashMap<u32, Vec<(u64, u64, u32, Vec<u8>)>>,
    serial: u64,
    pub sent: u64,
    pub bytes_sent: u64,
}

impl JitterLoopback {
    pub fn new(latency_frames: u64, jitter_frames: u64, seed: u64) -> Self {
        JitterLoopback { latency_frames, jitter_frames, loss: 0.0, lost: 0, frame: 0, seed: seed.max(1), queues: HashMap::new(), serial: 0, sent: 0, bytes_sent: 0 }
    }
    fn rand(&mut self) -> u64 {
        let mut x = self.seed;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.seed = x;
        x
    }
}

impl Transport for JitterLoopback {
    fn send(&mut self, to: u32, msg: &Msg, from: u32) {
        let b = msg.encode();
        if self.loss > 0.0 && (self.rand() >> 11) as f64 / (1u64 << 53) as f64 * 1.0 < self.loss {
            self.lost += 1;
            return;
        }
        let j = if self.jitter_frames > 0 { self.rand() % (self.jitter_frames + 1) } else { 0 };
        self.sent += 1;
        self.bytes_sent += b.len() as u64;
        self.serial += 1;
        let due = self.frame + self.latency_frames + j;
        self.queues.entry(to).or_default().push((due, self.serial, from, b));
    }
    fn receive(&mut self, at: u32) -> Vec<(u32, Msg)> {
        let frame = self.frame;
        let Some(q) = self.queues.get_mut(&at) else { return vec![] };
        let mut due: Vec<(u64, u64, u32, Vec<u8>)> = vec![];
        q.retain(|e| {
            if e.0 <= frame {
                due.push(e.clone());
                false
            } else {
                true
            }
        });
        due.sort_by_key(|e| (e.0, e.1));
        due.into_iter().filter_map(|(_, _, f, b)| Msg::decode(&b).map(|m| (f, m))).collect()
    }
    fn advance(&mut self) {
        self.frame += 1;
    }
    fn frame(&self) -> u64 {
        self.frame
    }
    fn stats(&self) -> (u64, u64) {
        (self.sent, self.bytes_sent)
    }
}

// ---- UDP: UE packets and bunches ------------------------------------------------------------------------------------
// A datagram is a UNetConnection packet (rust-net r8), bit-packed with wire.rs's FBitWriter:
//   - bHandshakePacket bit, and while this end has heard nothing from the peer its endpoint id (SerializeIntPacked)
//     (the StatelessConnectHandlerComponent's leading bit: UE 4.26 structure, UNCONFIRMED here; the id is this
//     transport's connection identity);
//   - FNetPacketNotify::WriteHeader rva=0x32233b0 (via UNetConnection::WritePacketHeader rva=0x3223540): one 32-bit word
//     ((AckedSeq & 0x3fff) << 14 | (Seq & 0x3fff)) << 4 | (HistoryWordCount - 1) (0x143223432..0x14322346a), then
//     HistoryWordCount 32-bit history words (at most 8, 0x1432233d8..0x143223410);
//   - bHasPacketInfoPayload bit, then (when set) bHasServerFrameTime (UNetConnection::ReadPacketInfo rva=0x3217d40);
//   - the bunches, each with the header UNetConnection::SendRawBunch rva=0x321d920 writes: bControl (= bOpen | bClose),
//     [bOpen, bClose, [CloseReason SerializeInt(15)]], bIsReplicationPaused, bReliable, ChIndex SerializeIntPacked,
//     bHasPackageMapExports, bHasMustBeMappedGUIDs, bPartial, [ChSequence WriteIntWrapped(1024) when reliable],
//     [ChName StaticSerializeName when reliable or opening], BunchDataBits WriteIntWrapped(MaxPacket * 8), the data;
//   - a final 1 bit (the packet's end marker; the reader finds the last set bit).
// Reliable bunches are resent when the packet that carried them is NAKed (the peer's ack history shows it missing),
// delivered in ChSequence order per channel; unreliable bunches are delivered on arrival, dropped on a channel not yet
// open. A connection with nothing to send sends a packet every KeepAliveTime 0.2 s (BaseEngine.ini
// [/Script/OnlineSubsystemUtils.IpNetDriver]) so acks keep flowing. A frame's messages to one actor share a bunch (as
// UActorChannel::ReplicateActor rva=0x3038b60 writes an actor's changes in one bunch).
// UNCONFIRMED (docs/RUST_NET.md section 10): MaxPacket 1024 (UE 4.26 MAX_PACKET_SIZE), the history bit order,
// channel indices assigned per direction (the engine's actor channels are the server's), ChSequence starting at 0,
// the EName indices of Actor (102) / Control (255), no partial bunches (every bunch here fits a packet).

const MAX_PACKET: usize = 1024;
const MAX_CHSEQUENCE: u32 = 1024;
const SEQ_MASK: u16 = 0x3fff;
const NAME_ACTOR: u32 = 102;
const NAME_CONTROL: u32 = 255;
const KEEP_ALIVE: f64 = 0.2;

/// Simulated network conditions on this endpoint's outgoing datagrams (tests): one-way delay latency + uniform
/// 0..jitter, and a loss probability applied to every datagram
#[derive(Clone, Copy, Debug, Default)]
pub struct NetSim {
    pub latency_ms: f64,
    pub jitter_ms: f64,
    pub loss: f64,
    pub seed: u64,
}

#[derive(Clone, Debug)]
struct OutBunch {
    ch: u32,
    reliable: bool,
    open: bool,
    close: bool,
    seq: u32,
    data: crate::wire::BitWriter,
    /// the subject a newly opened channel names (written at the start of the open bunch's data)
    subject: String,
}

#[derive(Clone, Debug)]
struct SentPacket {
    seq: u16,
    at: Instant,
    reliable: Vec<OutBunch>,
    /// our incoming sequence this packet acknowledged (FNetPacketNotify's InAckSeqAck once it is acked)
    in_seq_at_send: u16,
}

#[derive(Default)]
struct Peer {
    addr: Option<SocketAddr>,
    heard: bool,
    out_seq: u16,
    sent: VecDeque<SentPacket>,
    /// the highest of our sequences the peer acknowledged (initially "none")
    acked: Option<u16>,
    /// incoming: the newest sequence and the receive history (bit k = packet in_seq - k arrived)
    in_seq: Option<u16>,
    in_history: [u32; 8],
    in_ack_seq_ack: u16,
    dirty_acks: bool,
    queue: Vec<OutBunch>,
    out_names: HashMap<String, u32>,
    out_ch_seq: HashMap<u32, u32>,
    next_ch: u32,
    in_names: HashMap<u32, String>,
    in_ch_seq: HashMap<u32, u32>,
    in_buf: HashMap<u32, BTreeMap<u32, (bool, Vec<u8>, usize)>>,
    last_send: Option<Instant>,
}

/// One machine's UDP socket: UE connections to the other endpoints it knows (a client knows the server; the server
/// learns each client from its handshake packets).
pub struct UdpEndpoint {
    pub id: u32,
    sock: UdpSocket,
    peers: BTreeMap<u32, Peer>,
    inbox: VecDeque<(u32, Msg)>,
    /// smoothed round trip from packet acks (s)
    pub srtt: f64,
    pub sim: NetSim,
    rng: u64,
    delayed: Vec<(Instant, SocketAddr, Vec<u8>)>,
    pub sent: u64,
    pub bytes_sent: u64,
    /// bytes sent to each peer (datagram payloads; add `packets_to` x 28 for the IPv4 + UDP headers)
    pub bytes_to: HashMap<u32, u64>,
    pub packets_to: HashMap<u32, u64>,
    pub delivered: u64,
    pub resends: u64,
    pub dropped_by_sim: u64,
    /// packets or bunches that did not decode (dropped)
    pub undecodable: u64,
    frame: u64,
}

fn seq_newer(a: u16, b: u16) -> bool {
    let d = a.wrapping_sub(b) & SEQ_MASK;
    d != 0 && d < 0x2000
}
fn seq_delta(a: u16, b: u16) -> u16 {
    a.wrapping_sub(b) & SEQ_MASK
}
fn chseq_newer(a: u32, b: u32) -> bool {
    let d = a.wrapping_sub(b) % MAX_CHSEQUENCE;
    d != 0 && d < MAX_CHSEQUENCE / 2
}

impl UdpEndpoint {
    /// bind `addr` (e.g. "127.0.0.1:0" for any free port, "0.0.0.0:7777" for a server)
    pub fn bind(id: u32, addr: &str) -> io::Result<Self> {
        let sock = UdpSocket::bind(addr)?;
        sock.set_nonblocking(true)?;
        Ok(UdpEndpoint {
            id,
            sock,
            peers: BTreeMap::new(),
            inbox: VecDeque::new(),
            srtt: 0.25,
            sim: NetSim::default(),
            rng: 0x9e37_79b9_7f4a_7c15 ^ id as u64,
            delayed: Vec::new(),
            sent: 0,
            bytes_sent: 0,
            bytes_to: HashMap::new(),
            packets_to: HashMap::new(),
            delivered: 0,
            resends: 0,
            dropped_by_sim: 0,
            undecodable: 0,
            frame: 0,
        })
    }

    pub fn set_sim(&mut self, sim: NetSim) {
        self.sim = sim;
        if sim.seed != 0 {
            self.rng = sim.seed ^ ((self.id as u64) << 32) ^ 1;
        }
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.sock.local_addr()
    }

    pub fn add_peer(&mut self, id: u32, addr: SocketAddr) {
        let p = self.peers.entry(id).or_default();
        p.addr = Some(addr);
        if p.next_ch == 0 {
            p.next_ch = 1;
        }
    }

    fn rand01(&mut self) -> f64 {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x >> 11) as f64 / (1u64 << 53) as f64
    }

    /// every datagram goes out through the simulated network (or straight to the socket)
    fn raw_send(&mut self, d: Vec<u8>, a: SocketAddr) {
        if self.sim.loss > 0.0 && self.rand01() < self.sim.loss {
            self.dropped_by_sim += 1;
            return;
        }
        if self.sim.latency_ms > 0.0 || self.sim.jitter_ms > 0.0 {
            let ms = self.sim.latency_ms + self.sim.jitter_ms * self.rand01();
            self.delayed.push((Instant::now() + Duration::from_secs_f64(ms / 1000.0), a, d));
            return;
        }
        let _ = self.sock.send_to(&d, a);
    }

    /// queue a message on its subject's channel (a frame's messages to one actor and reliability share a bunch)
    fn queue(&mut self, to: u32, msg: &Msg, reliable: bool) {
        self.sent += 1;
        let subj = crate::wire::subject(msg);
        let p = self.peers.entry(to).or_default();
        if p.next_ch == 0 {
            p.next_ch = 1;
        }
        let close = matches!(msg, Msg::Destroy { .. });
        let (ch, open) = if subj.is_empty() {
            (0, false)
        } else {
            match p.out_names.get(&subj) {
                Some(&c) => (c, false),
                None => {
                    let c = p.next_ch;
                    p.next_ch = if p.next_ch >= 32766 { 1 } else { p.next_ch + 1 };
                    p.out_names.insert(subj.clone(), c);
                    (c, true)
                }
            }
        };
        // an opening bunch is reliable (the channel must exist before its unreliable traffic)
        let reliable = reliable || open || close || ch == 0;
        let mut item = crate::wire::BitWriter::new();
        item.serialize_int_packed(crate::wire::kind_of(msg));
        crate::wire::write_params(&mut item, msg);
        let last = p.queue.iter().rposition(|b| b.ch == ch);
        match last {
            Some(i) if p.queue[i].reliable == reliable && !p.queue[i].close && !close && p.queue[i].data.num + item.num < (MAX_PACKET - 64) * 8 => {
                p.queue[i].data.append(&item);
            }
            _ => {
                let seq = if reliable {
                    let s = p.out_ch_seq.entry(ch).or_insert(0);
                    let v = *s;
                    *s = (*s + 1) % MAX_CHSEQUENCE;
                    v
                } else {
                    0
                };
                p.queue.push(OutBunch { ch, reliable, open, close, seq, data: item, subject: if open { subj.clone() } else { String::new() } });
            }
        }
        if close {
            p.out_names.remove(&subj);
        }
    }

    pub fn send_msg(&mut self, to: u32, msg: &Msg) {
        self.queue(to, msg, true);
    }

    /// an unreliable message (ServerMove, the move answers, ReplicatedMovement in the engine's unreliable bunches)
    pub fn send_unreliable(&mut self, to: u32, msg: &Msg) {
        self.queue(to, msg, false);
    }

    fn write_bunch(w: &mut crate::wire::BitWriter, b: &OutBunch) {
        // UNetConnection::SendRawBunch rva=0x321d920 (header, see the section docs)
        w.write_bit(b.open || b.close);
        if b.open || b.close {
            w.write_bit(b.open);
            w.write_bit(b.close);
            if b.close {
                w.serialize_int(0, 15); // CloseReason: Destroyed
            }
        }
        w.write_bit(false); // bIsReplicationPaused
        w.write_bit(b.reliable);
        w.serialize_int_packed(b.ch);
        w.write_bit(false); // bHasPackageMapExports
        w.write_bit(false); // bHasMustBeMappedGUIDs
        w.write_bit(false); // bPartial
        if b.reliable {
            w.write_int_wrapped(b.seq, MAX_CHSEQUENCE);
        }
        if b.reliable || b.open {
            // ChName: StaticSerializeName with the hardcoded name
            w.write_bit(true);
            w.serialize_int_packed(if b.ch == 0 { NAME_CONTROL } else { NAME_ACTOR });
        }
        let mut data = crate::wire::BitWriter::new();
        if b.open {
            data.fstring(&b.subject);
        }
        data.append(&b.data);
        w.write_int_wrapped(data.num as u32, (MAX_PACKET * 8) as u32);
        w.append(&data);
    }

    fn packet_header(&self, p: &Peer) -> (crate::wire::BitWriter, u16) {
        let mut w = crate::wire::BitWriter::new();
        w.write_bit(!p.heard);
        if !p.heard {
            w.serialize_int_packed(self.id);
        }
        let seq = p.out_seq;
        let ack = p.in_seq.unwrap_or(SEQ_MASK);
        let delta = seq_delta(ack, p.in_ack_seq_ack) as usize;
        let words = delta.div_ceil(32).clamp(1, 8);
        let word = ((((ack & SEQ_MASK) as u32) << 14 | (seq & SEQ_MASK) as u32) << 4) | ((words - 1) as u32 & 0xf);
        w.u32(word);
        for i in 0..words {
            w.u32(p.in_history[i]);
        }
        w.write_bit(false); // bHasPacketInfoPayload
        (w, seq)
    }

    fn send_packet(&mut self, to: u32, bunches: Vec<OutBunch>) {
        let (mut w, seq, addr, in_seq) = {
            let Some(p) = self.peers.get(&to) else { return };
            let Some(addr) = p.addr else { return };
            let (w, seq) = self.packet_header(p);
            (w, seq, addr, p.in_seq.unwrap_or(SEQ_MASK))
        };
        let mut reliable = vec![];
        for b in &bunches {
            Self::write_bunch(&mut w, b);
            if b.reliable {
                reliable.push(b.clone());
            }
        }
        w.write_bit(true); // the end marker
        let d = w.buf.clone();
        let now = Instant::now();
        {
            let p = self.peers.get_mut(&to).unwrap();
            p.out_seq = (p.out_seq + 1) & SEQ_MASK;
            p.sent.push_back(SentPacket { seq, at: now, reliable, in_seq_at_send: in_seq });
            p.dirty_acks = false;
            p.last_send = Some(now);
        }
        self.bytes_sent += d.len() as u64;
        *self.bytes_to.entry(to).or_insert(0) += d.len() as u64;
        *self.packets_to.entry(to).or_insert(0) += 1;
        self.raw_send(d, addr);
    }

    /// UNetConnection::FlushNet: the queued bunches go out in packets of at most MaxPacket bytes; a connection with
    /// pending acks or nothing sent for KeepAliveTime sends a packet without bunches
    fn flush_all(&mut self) {
        let now = Instant::now();
        let ids: Vec<u32> = self.peers.keys().copied().collect();
        for id in ids {
            let (queue, need) = {
                let p = self.peers.get_mut(&id).unwrap();
                if p.addr.is_none() {
                    continue;
                }
                let q = std::mem::take(&mut p.queue);
                let idle = p.last_send.map(|t| now.duration_since(t).as_secs_f64() >= KEEP_ALIVE).unwrap_or(true);
                let acks_due = p.dirty_acks && p.last_send.map(|t| now.duration_since(t).as_secs_f64() >= 1.0 / 120.0).unwrap_or(true);
                (q, idle || acks_due)
            };
            if queue.is_empty() {
                if need {
                    self.send_packet(id, vec![]);
                }
                continue;
            }
            let mut cur: Vec<OutBunch> = vec![];
            let mut bits = 200; // header room
            for b in queue {
                let cost = 64 + b.data.num + b.subject.len() * 8;
                if !cur.is_empty() && bits + cost > MAX_PACKET * 8 {
                    self.send_packet(id, std::mem::take(&mut cur));
                    bits = 200;
                }
                bits += cost;
                cur.push(b);
            }
            if !cur.is_empty() {
                self.send_packet(id, cur);
            }
        }
    }

    fn handle_packet(&mut self, d: &[u8], addr: SocketAddr) {
        // the end marker: the last set bit
        let Some(last) = d.iter().rposition(|&b| b != 0) else { return };
        let total = last * 8 + (7 - d[last].leading_zeros() as usize);
        let mut r = crate::wire::BitReader::new(d, total);
        let handshake = r.read_bit();
        let from = if handshake {
            let id = r.serialize_int_packed();
            self.peers.entry(id).or_default().addr.get_or_insert(addr);
            id
        } else {
            match self.peers.iter().find(|(_, p)| p.addr == Some(addr)).map(|(k, _)| *k) {
                Some(k) => k,
                None => {
                    self.undecodable += 1;
                    return;
                }
            }
        };
        let word = r.u32();
        let words = ((word & 0xf) + 1) as usize;
        let seq = ((word >> 4) & SEQ_MASK as u32) as u16;
        let acked = ((word >> 18) & SEQ_MASK as u32) as u16;
        let mut hist = [0u32; 8];
        for h in hist.iter_mut().take(words) {
            *h = r.u32();
        }
        if r.read_bit() {
            let _server_frame_time = r.read_bit();
        }
        let now = Instant::now();
        let mut renak: Vec<OutBunch> = vec![];
        {
            let p = self.peers.get_mut(&from).unwrap();
            if p.next_ch == 0 {
                p.next_ch = 1;
            }
            p.heard = true;
            // incoming sequence and history
            match p.in_seq {
                Some(s) if !seq_newer(seq, s) => {
                    return; // an old or duplicate packet: dropped (its reliable bunches come again after the NAK)
                }
                Some(s) => {
                    let sh = seq_delta(seq, s) as usize;
                    let mut bits = [false; 256];
                    for k in 0..256 {
                        bits[k] = p.in_history[k / 32] >> (k % 32) & 1 != 0;
                    }
                    let mut nb = [false; 256];
                    for k in 0..256 {
                        if k >= sh {
                            nb[k] = bits[k - sh];
                        }
                    }
                    nb[0] = true;
                    p.in_history = [0; 8];
                    for (k, v) in nb.iter().enumerate() {
                        if *v {
                            p.in_history[k / 32] |= 1 << (k % 32);
                        }
                    }
                }
                None => {
                    p.in_history = [0; 8];
                    p.in_history[0] = 1;
                }
            }
            p.in_seq = Some(seq);
            p.dirty_acks = true;
            // our packets the peer reports on
            if acked != SEQ_MASK || p.acked.is_some() {
                while let Some(sp) = p.sent.front() {
                    if seq_newer(sp.seq, acked) {
                        break;
                    }
                    let sp = p.sent.pop_front().unwrap();
                    let k = seq_delta(acked, sp.seq) as usize;
                    let got = k < words * 32 && hist[k / 32] >> (k % 32) & 1 != 0;
                    if got {
                        let sample = now.duration_since(sp.at).as_secs_f64();
                        self.srtt = 0.875 * self.srtt + 0.125 * sample;
                        p.in_ack_seq_ack = sp.in_seq_at_send;
                    } else {
                        renak.extend(sp.reliable);
                    }
                }
                p.acked = Some(acked);
            }
            // NAKed reliable bunches go again (same ChSequence)
            for b in renak.drain(..) {
                self.resends += 1;
                p.queue.push(b);
            }
        }
        // the bunches
        while r.left() > 1 && !r.overflow {
            let control = r.read_bit();
            let (mut open, mut close) = (false, false);
            if control {
                open = r.read_bit();
                close = r.read_bit();
                if close {
                    let _reason = r.serialize_int(15);
                }
            }
            let _paused = r.read_bit();
            let reliable = r.read_bit();
            let ch = r.serialize_int_packed();
            let _exports = r.read_bit();
            let _must = r.read_bit();
            let partial = r.read_bit();
            let chseq = if reliable { r.serialize_int(MAX_CHSEQUENCE) } else { 0 };
            if partial {
                let _ = (r.read_bit(), r.read_bit());
            }
            if reliable || open {
                let _ = r.name_or_index();
            }
            let nbits = r.serialize_int((MAX_PACKET * 8) as u32) as usize;
            if r.overflow || nbits > r.left() {
                self.undecodable += 1;
                return;
            }
            let data = r.read_bits(nbits);
            let p = self.peers.get_mut(&from).unwrap();
            if reliable {
                let expect = *p.in_ch_seq.entry(ch).or_insert(0);
                if chseq == expect || chseq_newer(chseq, expect) {
                    p.in_buf.entry(ch).or_default().insert(chseq, (open, data, nbits));
                }
                // deliver in order
                loop {
                    let expect = *p.in_ch_seq.get(&ch).unwrap_or(&0);
                    let Some((o, dd, nb)) = p.in_buf.get_mut(&ch).and_then(|b| b.remove(&expect)) else { break };
                    p.in_ch_seq.insert(ch, (expect + 1) % MAX_CHSEQUENCE);
                    let msgs = Self::decode_bunch(p, ch, o, &dd, nb);
                    match msgs {
                        Some(ms) => {
                            for m in ms {
                                self.delivered += 1;
                                self.inbox.push_back((from, m));
                            }
                        }
                        None => self.undecodable += 1,
                    }
                }
                if close {
                    // closed after its data is delivered
                    if p.in_buf.get(&ch).map(|b| b.is_empty()).unwrap_or(true) {
                        p.in_names.remove(&ch);
                        p.in_ch_seq.remove(&ch);
                        p.in_buf.remove(&ch);
                    }
                }
            } else if ch == 0 || p.in_names.contains_key(&ch) {
                match Self::decode_bunch(p, ch, open, &data, nbits) {
                    Some(ms) => {
                        for m in ms {
                            self.delivered += 1;
                            self.inbox.push_back((from, m));
                        }
                    }
                    None => self.undecodable += 1,
                }
            }
        }
    }

    fn decode_bunch(p: &mut Peer, ch: u32, open: bool, data: &[u8], nbits: usize) -> Option<Vec<Msg>> {
        let mut r = crate::wire::BitReader::new(data, nbits);
        if open {
            let s = r.fstring();
            p.in_names.insert(ch, s);
        }
        let who = if ch == 0 { String::new() } else { p.in_names.get(&ch).cloned()? };
        let mut out = vec![];
        while r.left() > 0 {
            let kind = r.serialize_int_packed();
            out.push(crate::wire::read_params(&mut r, kind, &who)?);
        }
        Some(out)
    }

    /// read every datagram waiting on the socket, deliver the bunches, resend what was NAKed, flush the queued bunches;
    /// release the simulated network's due datagrams
    pub fn pump(&mut self) {
        let now = Instant::now();
        if !self.delayed.is_empty() {
            let mut keep = Vec::new();
            for (t, a, d) in std::mem::take(&mut self.delayed) {
                if t <= now {
                    let _ = self.sock.send_to(&d, a);
                } else {
                    keep.push((t, a, d));
                }
            }
            self.delayed = keep;
        }
        let mut buf = vec![0u8; 65536];
        loop {
            let (n, addr) = match self.sock.recv_from(&mut buf) {
                Ok(x) => x,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::ConnectionReset => continue, // Windows: ICMP port unreachable
                Err(_) => break,
            };
            let d = buf[..n].to_vec();
            self.handle_packet(&d, addr);
        }
        self.flush_all();
    }

    pub fn take(&mut self) -> Vec<(u32, Msg)> {
        self.inbox.drain(..).collect()
    }

    /// reliable bunches not yet acknowledged (queued or in flight), plus simulated datagrams not yet released
    pub fn unacked(&self) -> usize {
        self.peers.values().map(|p| p.queue.iter().filter(|b| b.reliable).count() + p.sent.iter().map(|s| s.reliable.len()).sum::<usize>()).sum::<usize>() + self.delayed.len()
    }
}

/// A single endpoint is a Transport for its own machine (a separate server or client process): `send` must come from
/// this endpoint, `receive` pumps the socket.
impl Transport for UdpEndpoint {
    fn send(&mut self, to: u32, msg: &Msg, _from: u32) {
        self.send_msg(to, msg);
    }
    fn receive(&mut self, _at: u32) -> Vec<(u32, Msg)> {
        self.pump();
        self.take()
    }
    fn advance(&mut self) {
        self.frame += 1;
    }
    fn poll(&mut self) {
        self.pump();
    }
    fn frame(&self) -> u64 {
        self.frame
    }
    fn stats(&self) -> (u64, u64) {
        (self.sent, self.bytes_sent)
    }
}

/// Every endpoint of a session over real UDP sockets on 127.0.0.1, in one process (the GDScript ENet adapter's role):
/// `receive` waits until every message sent so far has arrived, so the lockstep session frame stays the same as over
/// the loopback with no latency.
pub struct UdpLocal {
    pub eps: BTreeMap<u32, UdpEndpoint>,
    in_flight: u64,
    pub timeout: Duration,
    frame: u64,
    pub sent: u64,
    pub bytes_sent: u64,
}

impl UdpLocal {
    /// the server endpoint 0 and `n_clients` client endpoints 1..=n, each on its own 127.0.0.1 port
    pub fn start(n_clients: u32) -> io::Result<Self> {
        let mut eps = BTreeMap::new();
        let mut server = UdpEndpoint::bind(0, "127.0.0.1:0")?;
        let saddr = server.local_addr()?;
        for i in 1..=n_clients {
            let mut c = UdpEndpoint::bind(i, "127.0.0.1:0")?;
            c.add_peer(0, saddr);
            server.add_peer(i, c.local_addr()?);
            eps.insert(i, c);
        }
        eps.insert(0, server);
        Ok(UdpLocal { eps, in_flight: 0, timeout: Duration::from_secs(2), frame: 0, sent: 0, bytes_sent: 0 })
    }

    fn settle(&mut self) {
        let start = Instant::now();
        loop {
            let mut delivered = 0u64;
            for ep in self.eps.values_mut() {
                let before = ep.delivered;
                ep.pump();
                delivered += ep.delivered - before;
            }
            self.in_flight = self.in_flight.saturating_sub(delivered);
            if self.in_flight == 0 || start.elapsed() >= self.timeout {
                break;
            }
            std::thread::sleep(Duration::from_micros(200));
        }
    }
}

impl Transport for UdpLocal {
    fn send(&mut self, to: u32, msg: &Msg, from: u32) {
        if let Some(ep) = self.eps.get_mut(&from) {
            let b0 = ep.bytes_sent;
            ep.send_msg(to, msg);
            self.sent += 1;
            self.bytes_sent += ep.bytes_sent - b0;
            self.in_flight += 1;
        }
    }
    fn receive(&mut self, at: u32) -> Vec<(u32, Msg)> {
        self.settle();
        self.eps.get_mut(&at).map(|e| e.take()).unwrap_or_default()
    }
    fn advance(&mut self) {
        self.frame += 1;
    }
    fn poll(&mut self) {
        self.settle();
    }
    fn frame(&self) -> u64 {
        self.frame
    }
    fn stats(&self) -> (u64, u64) {
        (self.sent, self.bytes_sent)
    }
}

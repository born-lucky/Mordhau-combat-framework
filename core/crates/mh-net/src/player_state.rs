//! The parts of AMordhauPlayerState the replication rules read: the ping median every ping compensation uses
//! (PingMedian), and the team (Team / ReplicatedTeam). One per controller, on every machine.
//! Sources: extract/native/decomp/AMordhauPlayerState.cpp (fields: extract/native/types/AMordhauPlayerState.h).
//! Port of godot/game/net/net_player_state.gd.

use crate::consts::*;
use crate::ue::cvtss2si;

#[derive(Clone, Debug)]
pub struct NetPlayerState {
    /// +0x328 MedianPings (ring buffer, seconds)
    pub median_pings: Vec<f32>,
    /// +0x338 MedianPingsSorted
    pub median_pings_sorted: Vec<f32>,
    /// +0x348 CurMedianPingIndex
    pub cur_median_ping_index: usize,
    /// +0x34c PingMedian (seconds; a PackedFloat32Array element in the reference)
    pub ping_median: f64,
    /// AMordhauPlayerState Team (int32; -1 = none)
    pub team: i32,
    /// ReplicatedTeam (uint8; Team + 1, 0 = none)
    pub replicated_team: u8,
    /// APlayerState +0x224 PlayerId (NetGameSession::register_player)
    pub player_id: i32,
    /// logged in with ?SpectatorOnly=1 (counted as a spectator)
    pub spectator: bool,
}

/// AMordhauPlayerState::AMordhauPlayerState rva=0x15b54c0: MedianPings.AddZeroed(121) (`ArrayNum + 0x79`, memset 0x1e4
/// bytes), MedianPingsSorted.AddZeroed(MedianPings.Num())
pub const MEDIAN_PINGS_NUM: usize = 0x79;

impl Default for NetPlayerState {
    fn default() -> Self {
        NetPlayerState {
            median_pings: vec![0.0; MEDIAN_PINGS_NUM],
            median_pings_sorted: vec![0.0; MEDIAN_PINGS_NUM],
            cur_median_ping_index: 0,
            ping_median: 0.0,
            team: -1,
            replicated_team: 0,
            player_id: -1,
            spectator: false,
        }
    }
}

impl NetPlayerState {
    pub fn new() -> Self {
        Self::default()
    }

    /// from AMordhauPlayerState::UpdatePing rva=0x1604560 (disassembled 0x141604560..0x14160464a), InPing in seconds:
    ///   APlayerState::UpdatePing(InPing) (engine: ExactPing / the replicated Ping byte; not modelled, only PingMedian
    ///   is read)
    ///   InPing = min(InPing, 1)
    ///   CurMedianPingIndex = (CurMedianPingIndex + 1) % MedianPings.Num(); MedianPings[CurMedianPingIndex] = InPing
    ///   MedianPingsSorted[i] = MedianPings[i] for every i
    ///   Algo::Sort(MedianPings)  <- the exe sorts MedianPings itself (rcx = [rbx+0x328] at 0x1416045ec), not the copy
    ///   PingMedian = MedianPingsSorted[CeilToInt(Num x 0.5)] (cvtss2si(-0.5 - (Num x 0.5) x 2) >> 1, negated)
    /// So the median is read from the copy taken before the sort, and the ring index then writes into a sorted array.
    /// With a steady ping p and 121 zero slots, PingMedian stays 0 until p fills the upper half of the array (index 61).
    pub fn update_ping(&mut self, in_ping: f64) {
        let in_ping = (in_ping.min(PING_SAMPLE_MAX as f64)) as f32;
        let n = self.median_pings.len();
        self.cur_median_ping_index = (self.cur_median_ping_index + 1) % n;
        self.median_pings[self.cur_median_ping_index] = in_ping;
        self.median_pings_sorted.copy_from_slice(&self.median_pings);
        self.median_pings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let half = n as f64 * PING_MEDIAN_HALF as f64;
        let idx = -(cvtss2si(PING_MEDIAN_ROUND as f64 - (half + half)) >> 1);
        self.ping_median = self.median_pings_sorted[idx as usize] as f64;
    }

    /// from AMordhauPlayerState::SetTeam rva=0x15fd480 (authority only, on a change): Team = NewTeam; ReplicatedTeam =
    /// 0 when NewTeam < 0 (unsigned > 0x7fffffff), else clamp(NewTeam + 1, 1, 0xff)
    pub fn set_team(&mut self, new_team: i32, authority: bool) {
        if !authority || self.team == new_team {
            return;
        }
        self.team = new_team;
        if new_team < 0 {
            self.replicated_team = 0;
            return;
        }
        self.replicated_team = (new_team as i64 + 1).clamp(1, 0xff) as u8;
    }

    /// AMordhauPlayerState::OnRep_ReplicatedTeam rva=0x15e6d30: Team = ReplicatedTeam - 1
    pub fn on_rep_replicated_team(&mut self) {
        self.team = self.replicated_team as i32 - 1;
    }

    /// MotionSystem.team convention of the game adapters (game/mode/mordhau_match.gd: a player state Team < 0 is 255,
    /// "no team")
    pub fn combat_team(&self) -> i64 {
        if self.team >= 0 {
            self.team as i64
        } else {
            255
        }
    }
}

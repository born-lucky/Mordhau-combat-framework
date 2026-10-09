//! AMordhauBeaconClient: the short-lived beacon connection a client opens to a server before joining it, to measure
//! the ping (server browser) or to reserve slots for its party (PlayFab entities) so the join cannot be raced. The
//! client half and the host half (the RPC _Implementations run on the server's beacon actor) over a transport
//! (endpoint 0 = the beacon host). Port of godot/game/net/net_beacon.gd.
//! Sources: extract/native/decomp/AMordhauBeaconClient.cpp, fields extract/native/types/AMordhauBeaconClient.h.
//! Engine-UNCONFIRMED: the beacon connection itself (AOnlineBeaconClient::InitClient rva=0x1386140 / the host's
//! handshake): here connect opens it at once and the host's accept calls the client's on_connected.
//! Every beacon RPC _Validate (ServerPing_Validate / ServerReserveSlots_Validate) is the folded `return true` at
//! rva=0x7bf3e0.

use crate::consts::*;
use crate::enums::*;
use crate::game_session::NetGameSession;
use crate::msg::{Entity, Msg};
use crate::transport::Transport;
use crate::ue::{cvtss2si, f32r};

#[derive(Clone, Debug)]
pub struct BeaconClient {
    pub id: u32,
    /// +0x2b0 Request (EBeaconRequest; -1 = none yet)
    pub request: i32,
    /// +0x2b8 PingStartTime (seconds)
    pub ping_start_time: f64,
    /// +0x2c0 PlayerEntities
    pub player_entities: Vec<Entity>,
    /// PingResponse delegate calls: milliseconds
    pub ping_responses: Vec<i32>,
    /// ReserveSlotsResponse delegate calls: (open slots, EReservationStatus)
    pub reservation_responses: Vec<(i32, u8)>,
    pub connected: bool,
}

impl BeaconClient {
    pub fn new(endpoint: u32) -> Self {
        BeaconClient { id: endpoint, request: -1, ping_start_time: 0.0, player_entities: vec![], ping_responses: vec![], reservation_responses: vec![], connected: false }
    }

    fn connect(&mut self, t: &mut dyn Transport) -> bool {
        self.connected = true;
        t.send(0, &Msg::BeaconConnect { id: self.id }, self.id);
        true
    }

    /// AMordhauBeaconClient::Ping rva=0x15128b0: Request = 0 (Ping), then AOnlineBeaconClient::InitClient(URL)
    pub fn ping(&mut self, t: &mut dyn Transport) -> bool {
        self.request = BEACON_REQUEST_PING;
        self.connect(t)
    }

    /// AMordhauBeaconClient::ReserveSlots rva=0x1514ec0: Request = 1 (ReserveSlots), PlayerEntities =
    /// InPlayerEntities; none -> false; else InitClient(URL)
    pub fn reserve_slots(&mut self, t: &mut dyn Transport, entities: &[Entity]) -> bool {
        self.request = BEACON_REQUEST_RESERVE_SLOTS;
        self.player_entities = entities.to_vec();
        if self.player_entities.is_empty() {
            return false;
        }
        self.connect(t)
    }

    /// AMordhauBeaconClient::OnConnected rva=0x15084c0: Request 1 -> ServerReserveSlots(PlayerEntities); Request 0 ->
    /// PingStartTime = FPlatformTime::Seconds, ServerPing(); any other value -> the base OnConnected (vtable +0x638)
    pub fn on_connected(&mut self, t: &mut dyn Transport, now_s: f64) {
        if self.request == BEACON_REQUEST_RESERVE_SLOTS {
            t.send(0, &Msg::ServerReserveSlots { entities: self.player_entities.clone() }, self.id);
        } else if self.request == BEACON_REQUEST_PING {
            self.ping_start_time = now_s;
            t.send(0, &Msg::ServerPing, self.id);
        }
    }

    /// AMordhauBeaconClient::ClientPong_Implementation rva=0x14f4ba0: ms = (float)((Seconds - PingStartTime) x 1000.0)
    /// (double math, then cvtpd2ps); PingResponse(RoundToInt(ms)) = cvtss2si(ms + ms + 0.5) >> 1
    pub fn client_pong(&mut self, now_s: f64) -> i32 {
        let ms = f32r((now_s - self.ping_start_time) * PONG_S_TO_MS);
        let r = (cvtss2si(f32r(f32r(ms + ms) + PONG_ROUND_HALF as f64)) >> 1) as i32;
        self.ping_responses.push(r);
        r
    }

    /// AMordhauBeaconClient::ClientNotifyReservationStatus_Implementation rva=0x14f4b80: ReserveSlotsResponse(OpenSlots,
    /// ReservationStatus) when bound
    pub fn client_notify_reservation_status(&mut self, open_slots: i32, status: u8) {
        self.reservation_responses.push((open_slots, status));
    }

    /// AMordhauBeaconClient::OnFailure rva=0x1508730: the base OnFailure, then ReserveSlotsResponse(0, Failure 2) -
    /// also for a ping request (the code does not look at Request)
    pub fn on_failure(&mut self) {
        self.connected = false;
        self.reservation_responses.push((0, RESERVATION_FAILURE));
    }

    pub fn receive(&mut self, t: &mut dyn Transport, now_s: f64) {
        t.poll();
        for (_, msg) in t.receive(self.id) {
            match msg {
                Msg::BeaconAccept => self.on_connected(t, now_s),
                Msg::BeaconRefused => self.on_failure(),
                Msg::ClientPong => {
                    self.client_pong(now_s);
                }
                Msg::ClientNotifyReservationStatus { open, status } => self.client_notify_reservation_status(open, status),
                _ => {}
            }
        }
    }
}

#[derive(Debug)]
pub struct BeaconHost<'a> {
    /// UMordhauUtilityLibrary::GetMordhauGameSession; None = none
    pub session: Option<&'a mut NetGameSession>,
    /// the beacon host listens (false: connections fail -> the client's OnFailure)
    pub accepting: bool,
}

impl<'a> BeaconHost<'a> {
    pub fn new(session: Option<&'a mut NetGameSession>) -> Self {
        BeaconHost { session, accepting: true }
    }

    /// AMordhauBeaconClient::ServerPing_Implementation rva=0x15155a0: ClientPong()
    pub fn server_ping(&mut self, t: &mut dyn Transport, from: u32) {
        t.send(from, &Msg::ClientPong, 0);
    }

    /// AMordhauBeaconClient::ServerReserveSlots_Implementation rva=0x15155f0 (`Client requested %d slots`):
    ///   PlayerEntities = InPlayerEntities; no game session -> status Failure (2)
    ///   else ReserveSlot for every entity (status Success 0); any failure -> status Full (1) and FreeSlot for every
    ///   entity that is not IsPlayFabPlayerPresent (so a party is reserved whole or not at all, players already on the
    ///   server keep theirs)
    ///   ClientNotifyReservationStatus(GetOpenSlots, status). UNCONFIRMED: with no session the exe still calls
    ///   GetOpenSlots on the null session (GetReservedSlots reads its GameInstance); 0 here.
    pub fn server_reserve_slots(&mut self, t: &mut dyn Transport, from: u32, entities: &[Entity], now_unix: i64) -> u8 {
        let mut status = RESERVATION_FAILURE;
        let mut open = 0;
        if let Some(s) = self.session.as_deref_mut() {
            status = RESERVATION_SUCCESS;
            for e in entities {
                if !s.reserve_slot(e, now_unix) {
                    status = RESERVATION_FULL;
                }
            }
            if status == RESERVATION_FULL {
                for e in entities {
                    if !s.is_play_fab_player_present(&e.id) {
                        s.free_slot(e);
                    }
                }
            }
            open = s.get_open_slots();
        }
        t.send(from, &Msg::ClientNotifyReservationStatus { open, status }, 0);
        status
    }

    pub fn receive(&mut self, t: &mut dyn Transport, now_unix: i64) {
        t.poll();
        for (from, msg) in t.receive(0) {
            match msg {
                Msg::BeaconConnect { .. } => {
                    let m = if self.accepting { Msg::BeaconAccept } else { Msg::BeaconRefused };
                    t.send(from, &m, 0);
                }
                Msg::ServerPing => self.server_ping(t, from),
                Msg::ServerReserveSlots { entities } => {
                    self.server_reserve_slots(t, from, &entities, now_unix);
                }
                _ => {}
            }
        }
    }
}

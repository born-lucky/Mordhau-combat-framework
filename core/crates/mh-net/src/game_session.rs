//! The server's AMordhauGameSession / AGameSession: who may log in (ApproveLogin, AtCapacity, the PreLogin around it),
//! the player counts the capacity test reads (AGameMode NumPlayers / NumSpectators / NumTravellingPlayers), PlayerId
//! assignment (RegisterPlayer), the PlayFab slot reservations a joining party makes through the beacon (ReserveSlot /
//! FreeSlot / GetOpenSlots), and the ban / mute lookups. Port of godot/game/net/net_game_session.gd.
//! Sources: extract/native/decomp/AMordhauGameSession.cpp (game module); the AGameSession / AGameMode / AGameModeBase /
//! UGameplayStatics engine functions are statically linked in the exe and were read by disassembly (capstone).
//! Field offsets: extract/native/types/AGameSession.h, AMordhauGameSession.h.
//!
//! Not ported (online backend, not gameplay): PlayFab login / heartbeat / server registration timers (the
//! NextUpdateServerTime refresh AllowJoin schedules), the rcon commands, the webhooks, lag reports, the inventory /
//! stats caches FreeSlot clears. AMordhauGameSession::Tick, RegisterServer and AddAdmin are folded `ret` stubs in this
//! client exe (ICF at 0x7bf350), so reservation expiry is not in it either.

use std::collections::{BTreeMap, BTreeSet};

use crate::consts::*;
use crate::enums;
use crate::msg::Entity;
use crate::player_state::NetPlayerState;
use crate::ue::{cvtss2si, f32r};

#[derive(Clone, Debug)]
pub struct NetGameSession {
    // ---- config ----
    /// AGameSession +0x220 MaxSpectators
    pub max_spectators: i32,
    /// +0x224 MaxPlayers
    pub max_players: i32,
    /// +0x22c MaxSplitscreensPerConnection (uint8)
    pub max_splitscreens_per_connection: i32,
    /// AMordhauGameSession +0x578 MaxSlots (constructor: 0; PostInitProperties sets it)
    pub max_slots: i32,
    /// +0x574 AdminSlots (constructor rva=0x157d510 writes 0)
    pub admin_slots: i32,
    /// +0x278 bAllowJoin (constructor writes true)
    pub allow_join: bool,
    /// +0x27b bMatchHasEnded
    pub match_has_ended: bool,
    /// CVarMaxPlayersOverride "net.MaxPlayersOverride": registered by `dynamic initializer for
    /// 'CVarMaxPlayersOverride'` rva=0x74b250 as an int CVar with default 0 (`xor r8d, r8d`)
    pub max_players_override: i32,
    pub net_mode: u8,
    // ---- AGameMode player counts (the game mode's, kept here: AtCapacity is their only net reader) ----
    /// AGameMode +0x2cc NumSpectators
    pub num_spectators: i32,
    /// +0x2d0 NumPlayers
    pub num_players: i32,
    /// +0x2dc NumTravellingPlayers
    pub num_travelling_players: i32,
    /// AGameSession::RegisterPlayer::NextPlayerID (a function-static int in .data at 0x14551cfd8, process-wide in the
    /// exe; one session per server process here, so it lives on the session)
    pub next_player_id: i32,
    // ---- PlayFab state ----
    /// UMordhauGameInstance ReservedSlots: entity key -> reservation time (unix s - offset)
    pub reserved_slots: BTreeMap<String, i64>,
    /// +0x3d0 PresentPlayers: PlayFab entity ids
    pub present_players: BTreeSet<String>,
    /// BannedPlayers: PlayFab id -> ban end (unix s, 0 = permanent)
    pub banned_players: BTreeMap<String, i64>,
    pub official_banned_players: BTreeMap<String, i64>,
    pub muted_players: BTreeMap<String, i64>,
    pub official_muted_players: BTreeMap<String, i64>,
    /// UPlayFabAPI +0x128: the server clock's offset subtracted from UtcNow (s)
    pub playfab_time_offset: i64,
    /// OnRequestKick broadcasts: (playfab id, reason)
    pub kick_requests: Vec<(String, String)>,
    /// OnRequestBan broadcasts: (playfab id, duration, reason)
    pub ban_requests: Vec<(String, i32, String)>,
    /// OnRequestUnban broadcasts
    pub unban_requests: Vec<String>,
}

impl Default for NetGameSession {
    /// AGameSession config from DefaultGame.ini [/Script/Engine.GameSession] (consts::SESSION_*), the constructor's
    /// fields, a dedicated server
    fn default() -> Self {
        NetGameSession {
            max_spectators: SESSION_MAX_SPECTATORS,
            max_players: SESSION_MAX_PLAYERS,
            max_splitscreens_per_connection: SESSION_MAX_SPLITSCREENS,
            max_slots: 0,
            admin_slots: 0,
            allow_join: true,
            match_has_ended: false,
            max_players_override: 0,
            net_mode: enums::NET_MODE_DEDICATED_SERVER,
            num_spectators: 0,
            num_players: 0,
            num_travelling_players: 0,
            next_player_id: 0,
            reserved_slots: BTreeMap::new(),
            present_players: BTreeSet::new(),
            banned_players: BTreeMap::new(),
            official_banned_players: BTreeMap::new(),
            muted_players: BTreeMap::new(),
            official_muted_players: BTreeMap::new(),
            playfab_time_offset: 0,
            kick_requests: Vec::new(),
            ban_requests: Vec::new(),
            unban_requests: Vec::new(),
        }
    }
}

// ---- options (UGameplayStatics, engine) -----------------------------------------------------------------------------
/// UGameplayStatics::ParseOption rva=0x30e3ab0: GrabOption rva=0x30d9430 takes the options one by one, separated by
/// L"?" (0x14448d228); GetKeyValue rva=0x30d7020 splits each at the first '='; the key compares case-insensitively
/// (FGenericPlatformStricmp::Stricmp). The first match wins; "" when the key is absent or has no value.
pub fn parse_option(options: &str, key: &str) -> String {
    for part in options.split('?').filter(|s| !s.is_empty()) {
        let (k, v) = match part.find('=') {
            Some(i) => (&part[..i], &part[i + 1..]),
            None => (part, ""),
        };
        if k.eq_ignore_ascii_case(key) {
            return v.to_string();
        }
    }
    String::new()
}

/// `_wtoi`: leading whitespace, an optional sign, then decimal digits; 0 without digits
fn wtoi(s: &str) -> i32 {
    let t = s.trim_start();
    let (neg, rest) = match t.as_bytes().first() {
        Some(b'-') => (true, &t[1..]),
        Some(b'+') => (false, &t[1..]),
        _ => (false, t),
    };
    let mut v: i64 = 0;
    for c in rest.bytes() {
        if !c.is_ascii_digit() {
            break;
        }
        v = (v * 10 + (c - b'0') as i64).min(i64::from(i32::MAX) + 1);
    }
    let v = if neg { -v } else { v };
    v.clamp(i32::MIN as i64, i32::MAX as i64) as i32
}

/// UGameplayStatics::GetIntOption rva=0x30d6f60: the option's value through _wtoi when it is not empty, else the
/// default
pub fn get_int_option(options: &str, key: &str, default_value: i32) -> i32 {
    let v = parse_option(options, key);
    if v.is_empty() {
        default_value
    } else {
        wtoi(&v)
    }
}

/// FPlayFabEntity::IsValid rva=0x1487dc0 (FPlayFabPlayerEntity::IsValid is its ICF alias): ID not empty (ArrayNum >
/// 1, the terminator counts) and Type != 0. UNCONFIRMED: the map key equality (FPlayFabPlayerEntity GetTypeHash /
/// operator== not traced) is taken as ID + Type.
pub fn entity_valid(e: &Entity) -> bool {
    !e.id.is_empty() && e.r#type != 0
}

pub fn entity_key(e: &Entity) -> String {
    format!("{}:{}", e.r#type, e.id)
}

impl NetGameSession {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- setup ----
    /// AMordhauGameSession::PostInitProperties rva=0x15a35b0: MaxSlots from the command line ("MaxSlots=",
    /// FParse::Value); a value < 1 becomes 16 (0x10); MaxPlayers = MaxSlots + AdminSlots (overwriting the ini
    /// MaxPlayers). command_line_max_slots: the parsed value, 0 when the command line has none.
    pub fn post_init_properties(&mut self, command_line_max_slots: i32) {
        self.max_slots = command_line_max_slots;
        if self.max_slots < 1 {
            self.max_slots = 16;
        }
        self.max_players = self.max_slots + self.admin_slots;
    }

    /// AGameSession::InitOptions rva=0x30df0e0 (from AGameModeBase::InitGame, the map URL's options): MaxPlayers =
    /// GetIntOption(Options, "MaxPlayers", MaxPlayers); MaxSpectators = GetIntOption(Options, "MaxSpectators",
    /// MaxSpectators)
    pub fn init_options(&mut self, options: &str) {
        self.max_players = get_int_option(options, "MaxPlayers", self.max_players);
        self.max_spectators = get_int_option(options, "MaxSpectators", self.max_spectators);
    }

    // ---- login ----
    /// AGameSession::AtCapacity rva=0x30cb0c0 (vtable +0x660 of AMordhauGameSession, not overridden):
    ///   net mode 0 (standalone) -> false
    ///   spectator: AGameMode::GetNumSpectators rva=0x30d78a0 (NumSpectators) >= MaxSpectators and (net mode != 2 or
    ///     AGameMode::GetNumPlayers rva=0x30d7800 > 0)
    ///   player: limit = net.MaxPlayersOverride when > 0, else MaxPlayers; limit > 0 and GetNumPlayers >= limit
    /// GetNumPlayers = NumPlayers + NumTravellingPlayers.
    pub fn at_capacity(&self, spectator: bool) -> bool {
        if self.net_mode == enums::NET_MODE_STANDALONE {
            return false;
        }
        if spectator {
            if self.num_spectators < self.max_spectators {
                return false;
            }
            return self.net_mode != enums::NET_MODE_LISTEN_SERVER || self.get_num_players() > 0;
        }
        let limit = if self.max_players_override > 0 { self.max_players_override } else { self.max_players };
        limit > 0 && self.get_num_players() >= limit
    }

    pub fn get_num_players(&self) -> i32 {
        self.num_players + self.num_travelling_players
    }

    /// AMordhauGameSession::ApproveLogin rva=0x1584680 calls AGameSession::ApproveLogin rva=0x30ca8e0:
    ///   AtCapacity(GetIntOption(Options, L"SpectatorOnly" at 0x1449ccf18, 0) == 1) -> L"Server full." (at 0x144360160)
    ///   GetIntOption(Options, L"SplitscreenCount" at 0x1449cdf40, 0) > MaxSplitscreensPerConnection -> L"Maximum
    ///     splitscreen players" (at 0x1449cdff0)
    ///   else "" (approved)
    pub fn approve_login(&self, options: &str) -> String {
        let spectator = get_int_option(options, "SpectatorOnly", 0) == 1;
        if self.at_capacity(spectator) {
            return "Server full.".into();
        }
        if get_int_option(options, "SplitscreenCount", 0) > self.max_splitscreens_per_connection {
            return "Maximum splitscreen players".into();
        }
        String::new()
    }

    /// AGameModeBase::PreLogin rva=0x30e5ae0 (AMordhauGameMode does not override it): a valid unique id of another
    /// online subsystem's type -> L"incompatible_unique_net_id" (at 0x1449ccd10); else GameSession->ApproveLogin
    /// (vtable +0x648). unique_id_type_ok: the id is invalid or of the default subsystem's type.
    pub fn pre_login(&self, options: &str, unique_id_type_ok: bool) -> String {
        if !unique_id_type_ok {
            return "incompatible_unique_net_id".into();
        }
        self.approve_login(options)
    }

    /// AGameMode::PostLogin rva=0x30e4ff0: MustSpectate -> ++NumSpectators; in seamless travel with the client's world
    /// not yet loaded -> ++NumTravellingPlayers; else ++NumPlayers. AGameMode::Logout rva=0x30e2310 undoes the same
    /// counter.
    pub fn post_login(&mut self, spectator: bool, travelling: bool) {
        if spectator {
            self.num_spectators += 1;
        } else if travelling {
            self.num_travelling_players += 1;
        } else {
            self.num_players += 1;
        }
    }

    pub fn logout(&mut self, spectator: bool, travelling: bool) {
        if spectator {
            self.num_spectators -= 1;
        } else if travelling {
            self.num_travelling_players -= 1;
        } else {
            self.num_players -= 1;
        }
    }

    /// AMordhauGameSession::RegisterPlayer rva=0x15a4c50 jumps to AGameSession::RegisterPlayer rva=0x30e7fe0: no
    /// controller -> nothing; PlayerState->SetPlayerId(NextPlayerID++) (APlayerState::SetPlayerId rva=0x15a7350),
    /// SetUniqueId, RegisterPlayerWithSession (online session bookkeeping, not ported). Returns the id given.
    pub fn register_player(&mut self, ps: &mut NetPlayerState) -> i32 {
        let id = self.next_player_id;
        self.next_player_id += 1;
        ps.player_id = id;
        id
    }

    // ---- join gate and slots ----
    /// AMordhauGameSession::AllowJoin rva=0x1582560: bAllowJoin = InAllowJoin (on a change it also schedules the
    /// server list update, not ported); AllowsJoin rva=0x15825c0 returns it.
    pub fn set_allow_join(&mut self, b: bool) {
        self.allow_join = b;
    }

    pub fn allows_join(&self) -> bool {
        self.allow_join
    }

    /// AMordhauGameSession::HandleMatchHasEnded rva=0x1599e20: bMatchHasEnded = true; AllowJoin(false) inlined;
    /// SubmitServerLagReports (not ported)
    pub fn handle_match_has_ended(&mut self) {
        self.match_has_ended = true;
        self.set_allow_join(false);
    }

    /// AMordhauGameSession::GetReservedSlots rva=0x1598cc0: the ReservedSlots count (0 without a GameInstance)
    pub fn get_reserved_slots(&self) -> i32 {
        self.reserved_slots.len() as i32
    }

    /// AMordhauGameSession::GetOpenSlots rva=0x1597ce0: max(MaxSlots - GetReservedSlots, 0)
    pub fn get_open_slots(&self) -> i32 {
        (self.max_slots - self.get_reserved_slots()).max(0)
    }

    /// AMordhauGameSession::ReserveSlot rva=0x15a6950:
    ///   invalid entity -> false
    ///   not yet reserved: !bAllowJoin -> false; reserved count < MaxSlots -> add it (UpdateServer); else false
    ///   reserved (already, or just now): its time = unix seconds - the PlayFab offset; true
    pub fn reserve_slot(&mut self, e: &Entity, now_unix: i64) -> bool {
        if !entity_valid(e) {
            return false;
        }
        let k = entity_key(e);
        if !self.reserved_slots.contains_key(&k) {
            if !self.allow_join || self.get_reserved_slots() >= self.max_slots {
                return false;
            }
        }
        self.reserved_slots.insert(k, now_unix - self.playfab_time_offset);
        true
    }

    /// AMordhauGameSession::FreeSlot rva=0x15945a0 (the slot half): invalid entity -> nothing; else the reservation is
    /// removed (UpdateServer). The player cache clearing and RemovePlayFabPlayer around it are online bookkeeping.
    pub fn free_slot(&mut self, e: &Entity) {
        if !entity_valid(e) {
            return;
        }
        self.reserved_slots.remove(&entity_key(e));
    }

    /// AMordhauGameSession::IsPlayFabPlayerPresent rva=0x159c090: id not empty and in PresentPlayers
    pub fn is_play_fab_player_present(&self, entity_id: &str) -> bool {
        !entity_id.is_empty() && self.present_players.contains(entity_id)
    }

    // ---- bans / mutes ----
    /// AMordhauGameSession::IsPlayerBanned rva=0x159c0d0: id not empty and in the server's list or the official list
    pub fn is_player_banned(&self, id: &str) -> bool {
        !id.is_empty() && (self.banned_players.contains_key(id) || self.official_banned_players.contains_key(id))
    }

    /// AMordhauGameSession::IsPlayerMuted rva=0x159c140: the same rule on the mute lists
    pub fn is_player_muted(&self, id: &str) -> bool {
        !id.is_empty() && (self.muted_players.contains_key(id) || self.official_muted_players.contains_key(id))
    }

    /// AMordhauGameSession::GetPlayerBanEndTime rva=0x1597f70: BannedPlayers' end, else OfficialBannedPlayers', else -1
    pub fn get_player_ban_end_time(&self, id: &str) -> i64 {
        if let Some(e) = self.banned_players.get(id) {
            return *e;
        }
        if let Some(e) = self.official_banned_players.get(id) {
            return *e;
        }
        -1
    }

    /// AMordhauGameSession::GetPlayerBanDuration rva=0x1597e00 (minutes):
    ///   end == 0 -> 0 (permanent); end <= now - offset -> -1 (not banned / expired)
    ///   else CeilToInt((offset - now + end) x 1/60) = -(cvtss2si(-0.5 - 2x) >> 1), x as float
    pub fn get_player_ban_duration(&self, id: &str, now_unix: i64) -> i32 {
        let end = self.get_player_ban_end_time(id);
        if end == 0 {
            return 0;
        }
        let now = now_unix - self.playfab_time_offset;
        if end <= now {
            return -1;
        }
        let x = f32r((self.playfab_time_offset - now_unix + end) as f64 * BAN_SECONDS_TO_MINUTES as f64);
        -(cvtss2si(f32r(BAN_ROUND_NEG_HALF as f64 - (x + x))) >> 1) as i32
    }

    /// AMordhauGameSession::KickPlayer rva=0x159c280 (by controller) / BanPlayer rva=0x15846a0 / UnbanPlayer
    /// rva=0x15ac950: with a listener bound, only broadcast OnRequestKick / OnRequestBan / OnRequestUnban (caller "No
    /// caller ID provided") and return false: the kick or ban itself is done by whatever listens (the server's admin
    /// backend, not in the exe). A kick needs the player state's valid PlayFab player. BanPlayerWithDuration
    /// rva=0x1584760 only returns false.
    pub fn kick_player(&mut self, id: &str, reason: &str) -> bool {
        if !id.is_empty() {
            self.kick_requests.push((id.into(), reason.into()));
        }
        false
    }

    pub fn ban_player(&mut self, id: &str, duration: i32, reason: &str) -> bool {
        self.ban_requests.push((id.into(), duration, reason.into()));
        false
    }

    pub fn unban_player(&mut self, id: &str) -> bool {
        self.unban_requests.push(id.into());
        false
    }
}

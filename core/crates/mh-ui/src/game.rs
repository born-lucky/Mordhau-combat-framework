//! The match world the in-match HUD reads: the game mode a map runs (UGameMapsSettings GameModeMapPrefixes,
//! DefaultEngine.ini `+GameModeMapPrefixes=(Name="FFA",GameMode=...)`), that mode's own HUD / GameState / PlayerState
//! Blueprint classes (the GameMode CDO's HUDClass / GameStateClass / PlayerStateClass, e.g. BP_DuelGameMode ->
//! BP_DuelHUD, BP_DuelGameState), and PlayerState objects the host fills from the sim (name, team, score, kills, deaths,
//! assists, ping, alive). The HUD Blueprints then read them exactly as in the game: BP_ScoreboardEntryParent:Refresh
//! Entry reads PlayerState.Kills / Deaths / Score / Assists / Ping (x4 ms) / Team / bIsAlive / GetPlayerName(),
//! BP_OneTeamScoreboard:Refresh Players Array reads GameState.PlayerArray, BP_MordhauHUD:SendMessageToKillFeed takes
//! (Killer PlayerState, KilledBy text, Victim PlayerState).

use crate::model::*;
use crate::vm::Vm;
use std::collections::HashMap;

/// the game mode Blueprint package for a map name ("FFA_Arena" -> BP_DeathmatchGameMode): the GameModeMapPrefixes
/// entry whose Name prefixes the map name (UGameMapsSettings::GetGameModeForMapName: map name starts with the prefix
/// followed by '_': UNCONFIRMED exact match rule)
pub fn mode_for_map(ini: &str, map: &str) -> Option<String> {
    let short = map.rsplit('/').next().unwrap_or(map);
    for l in ini.lines() {
        let Some(rest) = l.trim().strip_prefix("+GameModeMapPrefixes=(") else { continue };
        let name = rest.split("Name=\"").nth(1).and_then(|x| x.split('"').next()).unwrap_or("");
        let gm = rest.split("GameMode=").nth(1).and_then(|x| x.split([')', ',']).next()).unwrap_or("");
        if name.is_empty() || gm.is_empty() {
            continue;
        }
        if short.len() > name.len() && short[..name.len()].eq_ignore_ascii_case(name) && short.as_bytes()[name.len()] == b'_' {
            let pkg = gm.trim_matches('"').rsplit_once('.').map(|x| x.0).unwrap_or(gm);
            return Some(crate::kismet::content_path(pkg));
        }
    }
    None
}

/// a class-valued CDO property's Blueprint package (TSubclassOf: an object reference to the generated class)
pub fn class_pkg(v: &V) -> Option<String> {
    match v {
        V::Asset(a) if !a.package.is_empty() && !a.package.starts_with("/Script/") => Some(a.package.clone()),
        _ => None,
    }
}

/// One player as the host reports it
#[derive(Clone, Debug, Default)]
pub struct PlayerInfo {
    pub id: u64,
    pub name: String,
    /// team index (AMordhauPlayerState Team; FFA players use their own index / 0)
    pub team: i64,
    pub score: f64,
    pub kills: i64,
    pub deaths: i64,
    pub assists: i64,
    /// round-trip ms (APlayerState Ping is ms / 4, a byte)
    pub ping_ms: i64,
    pub alive: bool,
    pub local: bool,
}

/// the PlayerState objects by host id
#[derive(Default)]
pub struct Players {
    pub by_id: HashMap<u64, Id>,
    pub class: Option<String>,
}

impl Players {
    /// create / update PlayerStates and GameState.PlayerArray; the local one becomes PlayerController.PlayerState
    pub fn set(&mut self, vm: &mut Vm, ps: &[PlayerInfo]) {
        let mut arr = vec![];
        for p in ps {
            let id = match self.by_id.get(&p.id) {
                Some(&o) => o,
                None => {
                    let c = match self.class.as_deref().and_then(|c| vm.bp_class(c)) {
                        Some(c) => c,
                        None => vm.native_class("MordhauPlayerState"),
                    };
                    let o = vm.new_obj(c, &format!("PlayerState_{}", p.id));
                    self.by_id.insert(p.id, o);
                    o
                }
            };
            vm.set(id, "PlayerName", V::Str(p.name.clone()));
            vm.set(id, "PlayerNamePrivate", V::Str(p.name.clone()));
            vm.set(id, "Team", V::Int(p.team));
            vm.set(id, "Score", V::Float(p.score));
            vm.set(id, "Kills", V::Int(p.kills));
            vm.set(id, "Deaths", V::Int(p.deaths));
            vm.set(id, "Assists", V::Int(p.assists));
            vm.set(id, "Ping", V::Int((p.ping_ms / 4).clamp(0, 255)));
            vm.set(id, "bIsAlive", V::Bool(p.alive));
            vm.set(id, "PlayerId", V::Int(p.id as i64));
            if p.local {
                let pc = vm.world["pc"];
                vm.set(pc, "PlayerState", V::Obj(id));
                vm.world.insert("ps", id);
                let pawn = vm.world["pawn"];
                vm.set(pawn, "PlayerState", V::Obj(id));
            }
            arr.push(V::Obj(id));
        }
        let gs = vm.world["gs"];
        vm.set(gs, "PlayerArray", V::Array(arr));
    }

    pub fn get(&self, id: u64) -> V {
        self.by_id.get(&id).map(|&o| V::Obj(o)).unwrap_or_default()
    }
}

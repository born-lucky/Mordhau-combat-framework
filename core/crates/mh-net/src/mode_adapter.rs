//! `ServerMode` for mh-mode's GameMode (MordhauGameMode + the mode Blueprints: DU, TF, FFA, TDM, SKM, FL), the way
//! godot/game/net/net_duel_match.gd drives game/mode's DuelMode: the mode's "spawn_pawn" / "destroy_pawn" /
//! "kill_pawn" / "team" events become pawn channels and SetTeam on the server; ReplicatedRoomGame
//! (BP_DuelPlayerController, ST_DuelRoomGame {Team1Wins, Team2Wins, RoundInfo {Stage, Winner, StartTime}}) goes to
//! the owning client as a controller property, with the reference's dictionary keys. Not a game rule: glue.

use mh_mode::game_mode::{GameMode, RoomGame};
use serde_json::json;

use crate::duel_match::{ModeEvent, ServerMode};

/// the reference's room-game dictionary (DuelMode.room_game_dict: team1_wins, team2_wins, round {stage, winner,
/// start_time})
pub fn room_game_json(r: &RoomGame) -> serde_json::Value {
    json!({
        "team1_wins": r.team1_wins,
        "team2_wins": r.team2_wins,
        "round": {"stage": r.round.stage, "winner": r.round.winner, "start_time": r.round.start_time},
    })
}

impl ServerMode for GameMode {
    fn post_login(&mut self, ctrl: &str, player_id: i64) {
        let c = self.add_ctrl(ctrl, false);
        GameMode::post_login(self, c, player_id);
    }
    fn logout(&mut self, ctrl: &str) {
        if let Some(c) = self.by_name(ctrl) {
            GameMode::logout(self, c);
        }
    }
    fn tick(&mut self, dt: f64) {
        GameMode::tick(self, dt);
    }
    fn drain(&mut self) -> Vec<ModeEvent> {
        GameMode::drain(self)
            .into_iter()
            .map(|e| {
                let who = e.get("who").map(|a| a.as_s().to_string()).unwrap_or_default();
                match e.kind {
                    "spawn_pawn" => ModeEvent::SpawnPawn { who },
                    "destroy_pawn" => ModeEvent::DestroyPawn { who },
                    "kill_pawn" => ModeEvent::KillPawn { who },
                    "team" => ModeEvent::Team { who, team: e.get("team").map(|a| a.as_i()).unwrap_or(-1) },
                    k => ModeEvent::Other { kind: k.to_string(), data: e.to_json() },
                }
            })
            .collect()
    }
    fn set_alive(&mut self, ctrl: &str, alive: bool) {
        if let Some(c) = self.by_name(ctrl) {
            GameMode::set_alive(self, c, alive);
        }
    }
    fn team_of(&self, ctrl: &str) -> i64 {
        self.by_name(ctrl).map(|c| self.ctrls[c].team).unwrap_or(-1)
    }
    fn replicated_room_game(&self, ctrl: &str) -> Option<serde_json::Value> {
        let c = self.by_name(ctrl)?;
        self.ctrls[c].replicated_room_game.as_ref().map(room_game_json)
    }
    fn login_bot(&mut self, ctrl: &str) -> bool {
        // MordhauGameMode: a bot's AMordhauAIController logs in like a player without a connection (PlayerId -1)
        let c = self.add_ctrl(ctrl, true);
        GameMode::post_login(self, c, -1);
        true
    }
}

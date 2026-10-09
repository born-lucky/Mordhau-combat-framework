//! ModeTable (godot/game/mode/mode_table.gd): the ported game modes, one row each. The id is the mode's map prefix
//! (UGameMapsSettings GameModeMapPrefixes in DefaultEngine.ini 452-460; the metadata Prefix the menu filters maps by).
//! Menu order = this order.
//!   rooms  the room-based adapter (BP_DuelGameMode and its child BP_Group3v3GameMode): bots logged in like players,
//!          dead pawns kept until RestartRoom destroys them
//!   bots   the menu shows the Bot Count slider (hidden for room modes: a room holds MaxPeoplePerRoom)
//! Maps: <prefix>_Arena; SKM_Arena / TF_Arena fall back to a generated scene with the same streamed Arena sub-level
//! until the map pipeline builds them. Frontline (FL) is not listed: its map (FL_Camp) is not generated yet.
//! The host loads the row's mode data (ModeData) and builds the mode with GameMode::new.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModeRow {
    pub id: &'static str,
    /// the map package name the row plays on
    pub map: &'static str,
    /// a map to fall back to while `map` is not generated ("" = none)
    pub fallback: &'static str,
    pub rooms: bool,
    pub bots: bool,
}

pub const ROWS: [ModeRow; 5] = [
    ModeRow { id: "FFA", map: "FFA_Arena", fallback: "", rooms: false, bots: true },
    ModeRow { id: "TDM", map: "TDM_Arena", fallback: "", rooms: false, bots: true },
    ModeRow { id: "SKM", map: "SKM_Arena", fallback: "TDM_Arena", rooms: false, bots: true },
    ModeRow { id: "DU", map: "DU_Arena", fallback: "", rooms: true, bots: false },
    ModeRow { id: "TF", map: "TF_Arena", fallback: "DU_Arena", rooms: true, bots: false },
];

pub fn rows() -> &'static [ModeRow] {
    &ROWS
}

pub fn row(id: &str) -> Option<&'static ModeRow> {
    ROWS.iter().find(|r| r.id == id)
}

//! The Horde / Battle Royale Blueprint-function rules of the spec matrix (data_gen/spec/rules.json, `RULE_HRD_*` /
//! `RULE_BR_*`, docs/SPEC_SHEETS.md "Horde and Battle Royale rules") and where this crate implements each one
//! (rust-mode-ai r4). Machine-readable for the sheets builder's `implemented_by` / `status` columns; tests/rule_ids.rs
//! checks that the table covers exactly the spec's ids, and that every `implemented_by` function exists in the source.
//!
//! status: `ported` = the bytecode's game logic is in the named function and covered by the named test;
//! `host` = spatial / visual / actor-spawning work the engine-neutral crate hands to the host as an event (the
//! decision logic around it is ported); `not_ported` = no port (reason in the note).

/// (rule id, status, implemented_by "path::fn" (;-list), test, note)
pub const RULES: &[(&str, &str, &str, &str, &str)] = &[
    // ---- BP_HordeGameMode ----
    ("RULE_HRD_HordeGameMode_ExecuteUbergraph_BP_HordeGameMode", "ported", "src/horde.rs::horde_receive_tick;src/horde.rs::start_horde_match;src/horde.rs::horde_on_killed_pre;src/horde.rs::horde_on_killed_post;src/horde.rs::horde_post_login;src/horde.rs::horde_on_logout", "tests/horde_br.rs", "the events' ubergraph segments (ReceiveTick@7540, K2_PostLogin@9501, OnKilled, K2_OnLogout)"),
    ("RULE_HRD_HordeGameMode_ReceiveTick", "ported", "src/horde.rs::horde_receive_tick", "tests/horde_br.rs", ""),
    ("RULE_HRD_HordeGameMode_ReceiveBeginPlay", "ported", "src/horde.rs::new", "tests/horde_br.rs", "ubergraph entry only (HordeState::new holds the begin-play state)"),
    ("RULE_HRD_HordeGameMode_StartHordeMatch", "ported", "src/horde.rs::start_horde_match", "tests/horde_br.rs", ""),
    ("RULE_HRD_HordeGameMode_ProgressWave", "ported", "src/horde.rs::progress_wave", "tests/horde_br.rs", "vehicle-driver branch not ported (no vehicles)"),
    ("RULE_HRD_HordeGameMode_TriggerDefeat", "ported", "src/horde.rs::trigger_defeat", "tests/horde_br.rs", ""),
    ("RULE_HRD_HordeGameMode_SpawnSquadWave", "ported", "src/horde.rs::spawn_squad_wave", "tests/horde_br.rs", ""),
    ("RULE_HRD_HordeGameMode_Generate_Squads_in_Wave", "ported", "src/horde.rs::generate_squads_in_wave", "tests/horde_br.rs", ""),
    ("RULE_HRD_HordeGameMode_Find_Squad", "ported", "src/horde.rs::find_squad", "tests/horde_br.rs", ""),
    ("RULE_HRD_HordeGameMode_SpawnSquad", "ported", "src/horde.rs::spawn_squad", "tests/horde_br.rs", "the pawn spawn itself is a horde_spawn event for the host"),
    ("RULE_HRD_HordeGameMode_GetDifficultyRatio", "ported", "src/horde.rs::get_difficulty_ratio", "tests/horde_br.rs", ""),
    ("RULE_HRD_HordeGameMode_GetSpawnDelayRatio", "ported", "src/horde.rs::get_spawn_delay_ratio", "tests/horde_br.rs", ""),
    ("RULE_HRD_HordeGameMode_OnKilled", "ported", "src/horde.rs::horde_on_killed_pre;src/horde.rs::horde_on_killed_post;src/horde.rs::horde_enemy_killed", "tests/horde_br.rs", ""),
    ("RULE_HRD_HordeGameMode_K2_PostLogin", "ported", "src/horde.rs::horde_post_login", "tests/horde_br.rs", ""),
    ("RULE_HRD_HordeGameMode_HandlePostLogin", "ported", "src/horde.rs::horde_post_login", "tests/horde_br.rs", ""),
    ("RULE_HRD_HordeGameMode_K2_OnLogout", "ported", "src/horde.rs::horde_on_logout", "tests/horde_br.rs", ""),
    ("RULE_HRD_HordeGameMode_ClearSquadWaveSpawningData", "ported", "src/horde.rs::clear_squad_wave_spawning_data", "tests/horde_br.rs", ""),
    ("RULE_HRD_HordeGameMode_SortBySpawnOrderWeightMagnitude", "ported", "src/horde.rs::generate_squads_in_wave", "tests/horde_br.rs", "inlined at Generate Squads in Wave@553 / @939 (SortAndFilterArrayByFunction)"),
    ("RULE_HRD_HordeGameMode_Sort_by_Squad_Difficulty", "not_ported", "", "", "no caller in the decoded bytecode (Less_FloatFloat only); UNCONFIRMED dead code"),
    ("RULE_HRD_HordeGameMode_PrepareAIControllers", "host", "src/horde.rs::spawn_squad", "tests/horde_br.rs", "the controller pool's size (MaxEnemiesInWave, @42) decides possession; spawning the controllers is the host's"),
    ("RULE_HRD_HordeGameMode_SpawnEquipmentFor", "host", "src/horde.rs::spawn_squad", "tests/horde_br.rs", "the equipment index draws (RandomIntegerInRange) are ported; the actor spawn is the host's"),
    ("RULE_HRD_HordeGameMode_SetupCustomizationReplicationActors", "host", "", "", "cosmetic replication actors (CustomizationVariants count is in data.rs)"),
    ("RULE_HRD_HordeGameMode_SpawnWave", "not_ported", "", "", "legacy Waves path: IsSquadSpawningEnabled is true on the CDO, never reached"),
    // ---- BP_HordeGameState ----
    ("RULE_HRD_HordeGameState_ExecuteUbergraph_BP_HordeGameState", "ported", "src/game_mode.rs::match_result_for;src/horde_extras.rs::horde_actors_tick", "tests/horde_br.rs", "HandleMatchEndInfo@1541-@2068 and the begin-play purchasable registration"),
    ("RULE_HRD_HordeGameState_HandleMatchEndInfo", "ported", "src/game_mode.rs::match_result_for", "tests/horde_br.rs", "victory / defeat \"Reached Wave N\""),
    ("RULE_HRD_HordeGameState_ReceiveBeginPlay", "ported", "src/horde_extras.rs::add_chest", "tests/horde_br.rs", "ubergraph @2341: Purchasables AddUnique"),
    ("RULE_HRD_HordeGameState_ReceiveTick", "ported", "src/horde_extras.rs::horde_actors_tick", "tests/horde_br.rs", "ubergraph @2361"),
    ("RULE_HRD_HordeGameState_ShouldHideSpawnInfoText", "host", "", "", "constant Hide = true (HUD)"),
    ("RULE_HRD_HordeGameState_OnRep_ReplicatedHordeMatchInfo", "host", "src/horde.rs::horde_match_info_event", "tests/horde_br.rs", "the client's HUD update; the server publishes the match info as an event"),
    ("RULE_HRD_HordeGameState_IsHordePurchaseAllowed", "ported", "src/horde_extras.rs::is_horde_purchase_allowed", "tests/horde_br.rs", ""),
    ("RULE_HRD_HordeGameState_PurchaseHordeBuyMenuEntry", "ported", "src/horde_extras.rs::purchase_buy_menu_entry", "tests/horde_br.rs", ""),
    ("RULE_HRD_HordeGameState_ModifyBuyMenuEntryPrice", "ported", "src/horde_extras.rs::buy_menu_price", "tests/horde_br.rs", ""),
    ("RULE_HRD_HordeGameState_Is_Valid_Wearable_For_Player", "ported", "src/horde_extras.rs::is_horde_purchase_allowed", "tests/horde_br.rs", "wearable entries (BuyerView facts from the host)"),
    ("RULE_HRD_HordeGameState_Spawn_Purchased_Actor_For_Player", "host", "src/horde_extras.rs::purchase_buy_menu_entry", "tests/horde_br.rs", "the duplicate roll is ported; the actor spawn is a BuyOutcome for the host"),
    ("RULE_HRD_HordeGameState_UpdateNextPurchasableVisuals", "host", "", "", "widget visibility (MordhauWidget.OverrideVisible)"),
    ("RULE_HRD_HordeGameState_UpdateSellingVendors", "host", "", "", "vendor actor visuals"),
    ("RULE_HRD_HORDE_SPEC", "ported", "src/horde.rs;src/horde_extras.rs;src/demon_horde.rs;src/br.rs", "tests/horde_br.rs", "docs/HORDE_SPEC.md reviewed against the bytecode (r2 / r3 corrections appended there)"),
    // ---- BP_BattleRoyaleGameMode ----
    ("RULE_BR_BattleRoyaleGameMode_ExecuteUbergraph_BP_BattleRoyaleGameMode", "ported", "src/br.rs::br_receive_tick;src/br.rs::br_post_login;src/br.rs::br_on_killed_pre", "tests/horde_br.rs", ""),
    ("RULE_BR_BattleRoyaleGameMode_ReceiveTick", "ported", "src/br.rs::br_receive_tick", "tests/horde_br.rs", ""),
    ("RULE_BR_BattleRoyaleGameMode_ReceiveBeginPlay", "ported", "src/br.rs::br_receive_tick", "tests/horde_br.rs", "ubergraph entry only"),
    ("RULE_BR_BattleRoyaleGameMode_StartRoundStart", "ported", "src/br.rs::br_start_round_start", "tests/horde_br.rs", ""),
    ("RULE_BR_BattleRoyaleGameMode_StartRound", "ported", "src/br.rs::br_start_round", "tests/horde_br.rs", ""),
    ("RULE_BR_BattleRoyaleGameMode_EndRound", "ported", "src/br.rs::br_end_round", "tests/horde_br.rs", ""),
    ("RULE_BR_BattleRoyaleGameMode_OnKilled", "ported", "src/br.rs::br_on_killed_pre", "tests/horde_br.rs", ""),
    ("RULE_BR_BattleRoyaleGameMode_K2_PostLogin", "ported", "src/br.rs::br_post_login", "tests/horde_br.rs", ""),
    ("RULE_BR_BattleRoyaleGameMode_ControllerCanRestart", "ported", "src/br.rs::br_controller_can_restart", "tests/horde_br.rs", ""),
    ("RULE_BR_BattleRoyaleGameMode_MoreThanOnePlayerAlive", "ported", "src/br.rs::br_receive_tick", "tests/horde_br.rs", "inlined as the living count"),
    // ---- BP_BattleRoyaleGameState ----
    ("RULE_BR_BattleRoyaleGameState_ExecuteUbergraph_BP_BattleRoyaleGameState", "ported", "src/br.rs::br_receive_tick;src/game_mode.rs::match_result_for", "tests/horde_br.rs", ""),
    ("RULE_BR_BattleRoyaleGameState_HandleMatchEndInfo", "ported", "src/game_mode.rs::match_result_for", "tests/horde_br.rs", "winner-only victory"),
    ("RULE_BR_BattleRoyaleGameState_IsInGetReady", "ported", "src/br.rs::br_in_get_ready", "tests/horde_br.rs", ""),
    ("RULE_BR_BattleRoyaleGameState_ShouldBlockPawnInput", "ported", "src/br.rs::br_in_get_ready", "tests/horde_br.rs", "parent || IsInGetReady"),
    ("RULE_BR_BattleRoyaleGameState_OnRep_Countdown", "host", "src/br.rs::set_countdown", "tests/horde_br.rs", "the announcement text is the client's; the countdown value is ported"),
    ("RULE_BR_BattleRoyaleGameState_ReceiveTick", "ported", "src/br.rs::br_receive_tick", "tests/horde_br.rs", "ubergraph entry"),
    ("RULE_BR_BattleRoyaleGameState_ShouldHideSpawnInfoText", "host", "", "", "HUD constant"),
];

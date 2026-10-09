# sheets_contradictions.py - the spec's CONTRADICTED evidence (demo-parity FAIL / net-FAIL rows, state/parity/compare.json)
# grouped by rule/field and probable cause, with per-owner action lists -> state/spec_contradictions.md.
#   python scripts/sheets_contradictions.py
# The cause of each row is decided by `cause(row, ctx)` below from facts in the data, never by hand per row:
#   * server class (vanilla = our build 702625635; modded = NoChamber mod on our build; modded-other = demos from other
#     game builds, docs/PARITY.md §3c, not gated)
#   * whether the same metric PASSes on vanilla servers (then a modded-only failure is the mod, not the port)
#   * the spread of the deltas in the group (a mod script / data offset is one constant per metric; a wide spread is not)
#   * physically impossible observations (contact before the windup can end = extractor attribution, PARITY §6)
#   * secondary (net-) rows carry client input latency (PARITY §1: OnServerAssignNetMotion accepts client motions)
# sheets_populate.py imports cause() so the workbook's Evidence rows carry the same classification.
import collections, datetime, json, os, pathlib, statistics, sys

R = pathlib.Path(__file__).resolve().parents[1]
CMP = R / "state" / "parity" / "compare.json"
MOD = R / "state" / "parity" / "mod_json" / "Mordhau" / "Content" / "Mordhau" / "Maps" / "WeaponRemovalMap_VersionA" / "NoChamber"
OUT = R / "state" / "spec_contradictions.md"

# metric -> spec target (same table as sheets_populate.py) and the owner of the rule
TARGET = {"contact_time": "FLD_ATK_WINDUP", "feint_cost": "FLD_ATK_FEINT_COST", "morph_cost": "FLD_ATK_MORPH_COST",
          "miss_cost": "FLD_ATK_MISS_STAMINA_COST", "feint_lockout": "FLD_ATK_FEINT_LOCK_OUT",
          "windup_release": "RULE_ATTACK_TIMELINE", "windup_release_stamina": "RULE_STAMINA",
          "miss_total": "RULE_ATTACK_TIMELINE", "combo_total": "RULE_ATTACK_TIMELINE",
          "misscombo_total": "RULE_ATTACK_TIMELINE", "morph_total": "RULE_ATTACK_TIMELINE",
          "riposte_total": "RULE_ATTACK_TIMELINE", "feint_time": "RULE_ATTACK_WINDOWS", "morph_time": "RULE_ATTACK_WINDOWS",
          "morph_time_min": "RULE_ATTACK_WINDOWS", "flinch_total": "RULE_FLINCH", "blocked_p2": "RULE_BLOCKED",
          "blocked_total": "RULE_BLOCKED", "block_drain": "RULE_STAMINA", "riposte_delay": "RULE_PARRY",
          "parry_up": "RULE_PARRY", "parry_up_block": "RULE_PARRY", "parry_fail_recovery": "RULE_PARRY",
          "parry_miss_recovery": "RULE_PARRY", "parry_success_recovery": "RULE_PARRY"}
CONSTANT_SPREAD = {"s": 0.035, "pts": 0}   # max-min of deltas that still reads as one constant offset (35 ms ~ 2 ticks + noise)

PASSING = ("PASS", "net-PASS")
FAILING = ("FAIL", "net-FAIL")


def unit(metric):
    return "pts" if metric.endswith("_cost") or metric in ("blocked_p2", "block_drain") else "s"


def context(rows):
    ctx = {"vanilla_pass": collections.defaultdict(int), "vanilla_fail": collections.defaultdict(int),
           "group": collections.defaultdict(list), "mod_script": set()}
    for r in rows:
        m = r["metric"]
        if r["server"] == "vanilla":
            if r["status"] in PASSING: ctx["vanilla_pass"][m] += 1
            if r["status"] in FAILING: ctx["vanilla_fail"][m] += 1
        if r["status"] in FAILING and r.get("delta") is not None:
            ctx["group"][(r["server"], m)].append(r["delta"])
    # mod weapons whose Blueprint overrides gameplay events (the rewrite runs package data, not mod script)
    for f in sorted((MOD / "Weapons").glob("*.json")) if MOD.exists() else []:
        t = f.read_text(encoding="utf-8", errors="replace")
        if "ReceiveBeginPlay" in t or "OnBlocked" in t:
            ctx["mod_script"].add(f.stem)
    return ctx


def spread(ds):
    return (max(ds) - min(ds)) if ds else 0.0


def cause(r, ctx):
    """-> (cause id, owner, one-line reason)"""
    m, srv, st, d = r["metric"], r["server"], r["status"], r.get("delta")
    w = r["weapon"]
    if m == "respawn_delay":
        # PlayerRespawnTime is a config property: DefaultGame.ini sets it per game mode class
        # ([BP_PushGameMode_C] PlayerRespawnTime=1.0), so a server's own Game.ini can change it
        return ("server-config", "mode", "a respawn earlier than the class default PlayerRespawnTime: it is a config "
                "UPROPERTY (extract/config/DefaultGame.ini sets it for BP_PushGameMode), so the recording server's "
                "Game.ini most likely overrides it; not a port rule failure unless default-config demos show it")
    if srv == "modded-other":
        return ("other-build", "parity", "demo from another game build (state/parity/demo_builds.json, PARITY §3c): "
                "data differs from 702625635; informational, not gated")
    if st == "net-FAIL":
        return ("net-latency", "none", "secondary row: one end is a client input that crossed the network "
                "(OnServerAssignNetMotion accepts client motions, PARITY §1); not a rule failure")
    if m == "contact_time" and d is not None and d < 0:
        return ("extractor", "parity", "observed contact earlier than the windup can end is impossible: hit/parry "
                "event attributed to the wrong attack (PARITY §6 Billhook)")
    if srv == "modded":
        ds = ctx["group"][(srv, m)]
        const = spread(ds) <= CONSTANT_SPREAD[unit(m)]
        scripted = w in ctx["mod_script"] or w.endswith("_NC")
        tag = f"group delta {min(ds):+.3f}..{max(ds):+.3f}" if ds else "no delta"
        if ctx["vanilla_pass"][m] and not ctx["vanilla_fail"][m]:
            if const:
                return ("mod-script", "parity", f"PASSes on vanilla; NoChamber weapon Blueprint overrides "
                        f"ReceiveBeginPlay/OnBlocked (state/parity/mod_json), not run by the rewrite; {tag} = one constant")
            return ("mod-script-spread", "combat", f"PASSes on vanilla but the modded deltas spread ({tag}): mod script "
                    "most likely, but a weapon-dependent port term cannot be ruled out")
        if not ctx["vanilla_pass"][m]:
            return ("no-vanilla-evidence", "combat", f"no vanilla sample of this metric; {tag}. Same size as the mod's "
                    "recovery offset on miss/riposte totals (which PASS on vanilla), but unverified on the real game")
    if srv == "vanilla":
        return ("port-suspect", "combat", "primary vanilla row on our build: the port disagrees with the real game")
    return ("unclassified", "combat", "no rule matched")


OWNER_ACTIONS = {
    "combat": "godot/game/combat (reference) + core/crates/mordhau-core (Rust)",
    "parity": "parity harness (scripts/parity, docs/PARITY.md; orchestrator)",
    "character": "godot/game/character + mh-character",
    "mode": "godot/game/mode + mh-mode",
    "none": "no action (measurement is informational)",
}


CHAR_MODE = R / "state" / "parity" / "char_mode.json"
TARGET.update({"respawn_delay": "FLD_MODE_SCORING_PLAYER_RESPAWN_TIME", "skm_round_start": "FLD_MODE_ROUND_START_DURATION",
               "skm_round_end": "FLD_MODE_ROUND_END_DURATION", "skm_round_play_max": "FLD_MODE_ROUND_DURATION",
               "skm_late_spawn": "FLD_MODE_LATE_ROUND_SPAWN_DURATION",
               "move_sprint_ratio": "FLD_MOV_SPRINT_MODIFIER", "move_diag_sprint_ratio": "FLD_MOV_SPRINT_MODIFIER",
               "move_backpedal_ratio": "FLD_MOV_BACKPEDAL_MODIFIER", "move_crouch_ratio": "FLD_MOV_MAX_WALK_SPEED_CROUCHED",
               "move_strafe_ratio": "ENT_MOV_MOVEMENT", "jump_gravity": "FLD_MOV_GRAVITY_SCALE",
               "jump_takeoff_vz": "FLD_MOV_JUMP_Z_VELOCITY", "jump_apex": "FLD_MOV_JUMP_Z_VELOCITY",
               "jump_air_time": "FLD_MOV_JUMP_Z_VELOCITY"})
RUST_PARITY = R / "state" / "parity" / "rust_demo_parity.json"
MOVE_SPEC = R / "state" / "parity" / "movement_spec.json"
TARGET.update({"supersprint_speed": "FLD_MOV_SUPERSPRINT_MODIFIER", "chase_speed": "FLD_MOV_CHASING_MODIFIER",
               "team_count": "FLD_MODE_STATE_TEAM_COUNT", "is_team_mode": "FLD_MODE_STATE_B_IS_TEAM_MODE",
               "bot_footwork": "FLD_BOT_WILL_FOOTWORK", "sprint_ratio_spec": "FLD_MOV_SPRINT_MODIFIER",
               "backpedal_ratio_spec": "FLD_MOV_BACKPEDAL_MODIFIER", "crouch_ratio_spec": "FLD_MOV_MAX_WALK_SPEED_CROUCHED",
               "jump_vz_spec": "FLD_MOV_JUMP_Z_VELOCITY"})


def load_rows():
    """compare.json + char_mode.json (scripts/parity/char_mode_metrics.py) + rust_demo_parity.json (rust-character's
    exe-mode movement parity, scripts/parity/rust_demo_parity.py) rows, each tagged with its file"""
    rows = []
    for p in (CMP, CHAR_MODE, RUST_PARITY, MOVE_SPEC):
        if p.exists():
            for r in json.loads(p.read_text(encoding="utf-8"))["rows"]:
                r["_file"] = p.name
                rows.append(r)
    return rows


def main():
    if not CMP.exists():
        sys.exit("no state/parity/compare.json")
    rows = load_rows()
    ctx = context(rows)
    bad = [r for r in rows if r["status"] in FAILING]
    by_cause = collections.defaultdict(list)
    for r in bad:
        c = cause(r, ctx)
        by_cause[c].append(r)
    stamp = datetime.datetime.fromtimestamp(CMP.stat().st_mtime).strftime("%Y-%m-%d %H:%M")
    L = ["# Spec contradictions (demo parity vs the port)", "",
         f"Generated by `scripts/sheets_contradictions.py` from `state/parity/compare.json` ({stamp}); do not edit, rerun.",
         "Each row is a CONTRADICTED Evidence row of `sheets/mordhau_spec.xlsx` (status FAIL / net-FAIL). The cause is "
         "decided by rules over the data (script header), the per-row workbook observation carries the same cause.", "",
         f"**{len(bad)} contradicted rows.** Primary vanilla rows on our build that the port fails: "
         f"**{sum(1 for (c, _, _), v in by_cause.items() for _ in v if c == 'port-suspect')}**.", ""]
    if (datetime.datetime.now() - datetime.datetime.fromtimestamp(CMP.stat().st_mtime)).days >= 1 or \
            os.environ.get("SHEETS_PARITY_STALE"):
        L += [f"> STALE INPUT: compare.json is from {stamp}; combat rounds r7-r12 (state/log.md) changed the port since. "
              "Rerun the parity pipeline (docs/PARITY.md §5) before acting on any combat row below.", ""]
    L += ["## By probable cause", "", "| cause | owner | rows | metrics | reason (example) |", "|---|---|---:|---|---|"]
    agg = collections.defaultdict(lambda: [0, set(), ""])
    for (c, o, why), v in by_cause.items():
        a = agg[(c, o)]
        a[0] += len(v)
        a[1].update(r["metric"] for r in v)
        a[2] = a[2] or why
    for (c, o), (n, ms, why) in sorted(agg.items(), key=lambda kv: -kv[1][0]):
        L.append(f"| {c} | {o} | {n} | {', '.join(sorted(ms))} | {why} |")
    L += ["", "## By rule / field (spec target)", "", "| target | metric | server | status | rows | delta range | cause |",
          "|---|---|---|---|---:|---|---|"]
    grp = collections.defaultdict(list)
    for (c, o, _), v in by_cause.items():
        for r in v:
            grp[(TARGET.get(r["metric"], "?"), r["metric"], r["server"], r["status"], c)].append(r)
    for (t, m, s, st, c), v in sorted(grp.items()):
        ds = [r["delta"] for r in v if r.get("delta") is not None]
        rng = f"{min(ds):+.3f}..{max(ds):+.3f}" if ds else ""
        L.append(f"| {t} | {m} | {s} | {st} | {len(v)} | {rng} | {c} |")
    L += ["", "## Actions per owner", ""]
    acts = {
        "combat": [
            "Rerun parity first (input is stale). Then, for every `port-suspect` row (none in this input): fix the rule, "
            "add a test with the demo value as the expected value.",
            "`no-vanilla-evidence` (combo_total, misscombo_total, morph_total): the port has no real-game check of these "
            "timelines. The modded deltas match the mod's constant recovery offset seen on miss_total/riposte_total "
            "(which PASS on vanilla), so they are probably the mod, but they are UNCONFIRMED until vanilla demos with "
            "combos/morph-to-miss exist (ask the user for vanilla recordings).",
            "`mod-script-spread` rows: if the deltas stay weapon-dependent after the mod's BeginPlay script is known "
            "(parity action 1), look for a weapon-dependent term in the blocked / parry recovery path.",
            "Rust (mordhau-core): read spec floats as f32 (docs/SPEC_SHEETS.md 'Numbers') before comparing to these rows.",
        ],
        "parity": [
            "1. Decompile the NoChamber mod's weapon/character Blueprint bytecode (ReceiveBeginPlay, OnBlocked of BP_*_NC, "
            "BP_MorhauCharacter) from the mod pak (mdx decomp, one at a time) and model it in the parity overlay, or "
            "mark modded rows informational like modded-other. That resolves every `mod-script` row "
            f"({sum(len(v) for (c, _, _), v in by_cause.items() if c == 'mod-script')} rows).",
            "2. Rerun scripts/parity (compare.json predates combat r7-r12).",
            "3. `extractor` rows: drop contact samples earlier than the attacker's minimum possible windup end.",
            "4. `other-build` rows: keep informational (or re-decode against those builds' data).",
        ],
        "character": ["No character metric is measurable from the decoded demos yet: positions / velocities are not "
                      "decoded (speed, jump, fall damage), and the replicated stamina / health bytes are too sparse to see "
                      "regen ticks (scripts/parity/char_mode_metrics.py REGEN, off). Needs a decoder pass that keeps "
                      "ReplicatedMovement and every stat update (parity harness)."],
        "mode": ["respawn_delay (scripts/parity/char_mode_metrics.py, FFA/TDM): `server-config` rows mean the recording "
                 "servers respawn earlier than the class default PlayerRespawnTime. Confirm with demos from a server on "
                 "the default Game.ini before changing anything; the port reads the class default as it should.",
                 "Round / warmup timers, ticket drain and capture rate need the game state's replicated properties, "
                 "which decode_motions.py does not keep yet (parity harness)."],
    }
    for o, lst in acts.items():
        L += [f"### {o}: {OWNER_ACTIONS[o]}", ""] + [f"- {a}" for a in lst] + [""]
    L += ["## Rows", "", "| cause | server | weapon | kind | metric | status | observed | rewrite | delta | n |",
          "|---|---|---|---|---|---|---|---|---|---:|"]
    for (c, o, _), v in sorted(by_cause.items()):
        for r in sorted(v, key=lambda r: (r["server"], r["metric"], r["weapon"])):
            obs = r.get("obs", {})
            ov = obs.get("mode", obs.get("mean", obs.get("p99", "")))
            L.append(f"| {c} | {r['server']} | {r['weapon']} | {r['kind']} | {r['metric']} | {r['status']} | {ov} | "
                     f"{r.get('rewrite', '')} | {'' if r.get('delta') is None else round(r['delta'], 4)} | {r.get('n', '')} |")
    OUT.write_text("\n".join(L) + "\n", encoding="utf-8")
    cnt = collections.Counter(c for (c, _, _), v in by_cause.items() for _ in v)
    print(f"wrote {OUT.relative_to(R)}: {len(bad)} rows: " + ", ".join(f"{k} {v}" for k, v in cnt.most_common()))


if __name__ == "__main__":
    main()

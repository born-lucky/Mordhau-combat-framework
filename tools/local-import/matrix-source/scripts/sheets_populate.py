# sheets_populate.py - the Spreadsheet Method workbook (docs/methods/SPREADSHEET-METHOD.md, docs/SPEC_SHEETS.md),
# POPULATED BY SCRIPT from cited sources, never typed in (DESIGN §1: data is read, never re-authored):
#   godot/data_gen/spec_src/*.json   the typed records the port reads (components/ue readers over the paks / extract/json
#                                    + native ctor replay), dumped by godot/tools/gen_spec_src.gd
#   components/ue/rdata/*.gd         every named .rdata literal (`_k(name, va, [functions])`), value from
#                                    extract/native/rdata.tsv (hash-pinned exe)
#   godot/data_gen/mode/mode_kismet.json   Blueprint bytecode literals (scripts/mode_kismet.py)
#   godot/game/**, components/ue/records/*.gd   every `PDB::Name rva=0x..` citation -> a Rules row (RULE_FN_<rva>)
#   godot/tests/**/*.gd              test files that cite a rule's rva / name or read a field -> test_id links
#   state/parity/compare.json        demo-parity rows -> Evidence (observed / contradicted / unknown)
#
#   python scripts/sheets_populate.py           write sheets/mordhau_spec.xlsx (only if its content changed) + report
#   python scripts/sheets_populate.py --check   exit 1 if the saved workbook differs from a fresh population
# Idempotent: the workbook is compared cell by cell with the saved one; unchanged content is not rewritten, and every
# change is reported per sheet (rows added / removed / changed, keyed by the row ID).
# Memory: inputs are read one file at a time (weapons.json ~13 MB is the largest); the workbook is written in
# openpyxl write-only mode.
import collections, json, math, pathlib, re, struct, sys

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parents[1] / "tools" / "spec-kit"))
from speckit import book as kit_book  # noqa: E402  (the game-agnostic Book, numbers and workbook I/O)
from speckit.adapters import FnAdapter, Pipeline  # noqa: E402
from speckit.book import (BLOB_CHUNK, F32_OVERFLOW, F32_TYPES, SHEET_COLS, cell, diff, f32, f32_canon,  # noqa: E402,F401
                          infer, merge_type, norm, read_book, sheets, sid, write_book)

R = pathlib.Path(__file__).resolve().parents[1]
SRC = R / "godot" / "data_gen" / "spec_src"
OUT = R / "sheets" / "mordhau_spec.xlsx"
VERSION = "Mordhau Steam build 702625635 (UE 4.26.2)"

# Godot Variant.Type (the record's declared type, gen_spec_src __types) -> spec type
GD_TYPES = {1: "bool", 2: "int", 3: "float", 4: "string", 5: "vec2", 6: "vec2", 9: "vec3", 10: "vec3", 20: "color",
            21: "string", 27: "json", 28: "json", 30: "int_list", 31: "int_list", 32: "float_list", 33: "float_list",
            34: "string_list"}
# entity type -> (ID code, record source files whose `var <field>` line documents the field)
TYPES = {
    "weapon": ("WPN", ["godot/game/combat/weapon_data.gd", "godot/components/ue/records/equipment_def.gd",
                       "godot/components/ue/records/combat_data.gd"]),
    "attack": ("ATK", ["godot/game/combat/attack_info.gd"]),
    "motion": ("MOT", ["godot/components/ue/records/motion_defs.gd"]),
    "character": ("CHR", ["godot/components/ue/records/character_data.gd"]),
    "movement": ("MOV", ["godot/components/ue/records/character_data.gd"]),
    "stat": ("STAT", ["godot/components/ue/records/character_data.gd"]),
    "camera": ("CAM", ["godot/components/ue/records/character_data.gd"]),
    "physics": ("PHYS", ["godot/game/character/mordhau_movement.gd"]),
    "curve": ("CURVE", ["godot/components/ue/records/combat_data.gd"]),
    "horde": ("HRDD", ["godot/tools/golden/mode.gd"]),
    "perk": ("PERK", ["godot/components/ue/records/ue_wearable.gd", "godot/components/ue/records/character_data.gd"]),
    "mode": ("MODE", ["godot/game/mode/mordhau_mode_data.gd", "godot/components/ue/records/mode_data.gd",
                      "godot/components/ue/records/combat_data.gd", "godot/game/mode/mode_table.gd",
                      "godot/game/mode/ffa_mode.gd", "godot/game/mode/tdm_mode.gd", "godot/game/mode/skm_mode.gd",
                      "godot/game/mode/duel_mode.gd", "godot/game/mode/tf_mode.gd", "godot/game/mode/fl_mode.gd"]),
    "map": ("MAP", ["godot/game/mode/mode_table.gd"]),
    "bot": ("BOT", ["godot/components/ue/records/bot_data.gd"]),
    "wearable": ("WEAR", ["godot/game/character/wearable_data.gd"]),
    "constant": ("CONST", []),
    "nav": ("NAV", ["godot/components/ue/records/mode_data.gd"]),
    "bt": ("BT", ["godot/components/ue/records/bot_data.gd"]),
    "setting": ("SET", ["godot/game/ui/user_settings.gd"]),
    "equipment": ("EQP", ["godot/components/ue/records/customization_data.gd"]),
    "anim": ("ANIM", ["godot/components/ue/records/anim_data.gd"]),
    "mode_class": ("MCLS", []),
    "squad": ("SQD", []),
    "skin": ("SKIN", ["godot/components/ue/records/ue_weapon.gd"]),
    "color": ("COLOR", ["godot/components/ue/records/ue_wearable.gd"]),
    "cosmetic": ("COSM", []),
    "sound_cue": ("CUE", ["godot/components/ue/ue_sound.gd"]),
    "sound_att": ("SATT", ["godot/components/ue/ue_sound.gd"]),
    "sound_conc": ("SCONC", []),
    "sound_event": ("SEVT", []),
    # mh-character's exe-mode records (sheets r10; dump: mh-host bin mh-spec-src -> spec_src/character_exe.json)
    "horse": ("HORSE", ["core/crates/mh-character/src/exe_horse.rs", "core/crates/mh-host/src/exe_records.rs"]),
    "projectile": ("PROJ", ["core/crates/mh-character/src/projectile.rs"]),
    "equipment_movement": ("EQMV", ["core/crates/mh-character/src/equipment.rs"]),
    "ladder": ("LADDER", ["core/crates/mh-host/src/exe_records.rs", "core/crates/mh-character/src/exe_ladder.rs"]),
}
# Units only where a source states them; everything else says so ("" + unit_source UNCONFIRMED)
ATK_HDR = "godot/game/combat/attack_info.gd header: 'Times are seconds, turn caps degrees/s, damage per armor tier [0..3]'"
UNITS = {
    ("attack", f): ("s", ATK_HDR) for f in ["windup", "combo_windup_increase", "miss_combo_extra_windup_increase",
                                           "release", "feint_lock_out", "miss_recovery"]}
UNITS.update({("attack", "turn_caps"): ("deg/s", ATK_HDR)})
UNITS.update({("attack", f): ("hp per armor tier [0..3]", ATK_HDR) for f in ["damage", "head_bonus", "leg_bonus"]})
STAMINA_NOTE = "godot/game/combat/attack_motion.gd / melee_hit.gd pass it to MotionSystem.offset_stamina (stamina points)"
UNITS.update({("attack", f): ("stamina", STAMINA_NOTE) for f in ["feint_cost", "chamber_feint_cost", "chamber_cost",
                                                                 "morph_cost", "miss_stamina_cost", "hit_stamina_reward"]})
# motion windows: compared with / added to motion time (seconds) by the attack rules
MOT_NOTE = "godot/game/combat/attack_motion.gd adds / compares it with motion time in seconds"
UNITS.update({("motion", f): ("s", MOT_NOTE) for f in [
    "feint_window", "morph_window", "chamber_window", "recovery_queue_window", "riposte_windup_can_parry_window",
    "hit_recovery", "clashed_recovery", "hit_stop_recovery", "min_windup_time_before_morphing", "max_morph_total_time",
    "morph_kick_extra_time", "clash_on_parry_follow_up_windup", "clash_on_parry_can_parry_window"]})
UNITS[("constant", "va")] = ("address", "extract/native/rdata.tsv va column")

ATTACK_SLOTS = ["strike", "second_strike", "stab", "second_stab", "couch", "second_couch", "kick", "second_kick", "bash",
                "base_strike", "base_second_strike", "base_stab"]


def load(name):
    p = SRC / f"{name}.json"
    if not p.exists():
        sys.exit(f"missing {p.relative_to(R)}: run godot/tools/gen_spec_src.gd first (docs/SPEC_SHEETS.md)")
    return json.loads(p.read_text(encoding="utf-8"))


class Book(kit_book.Book):
    """the spec-kit Book (tools/spec-kit/speckit/book.py) with this game's entity codes, units and build"""
    def __init__(self):
        super().__init__({t: c for t, (c, _) in TYPES.items()}, UNITS, VERSION)
        self.perk_info = []     # per perk: entity, EPerk value / name, its UPerkSystemComponent parameter names


def put_record(b, etype, eid, rec, skip=(), prefix=""):
    """every script variable of a dumped record -> a typed field (nested records flattened one level by prefix)"""
    types = rec.get("__types", {})
    for k, v in rec.items():
        if k.startswith("__") or k in skip:
            continue
        name = prefix + k
        if isinstance(v, dict) and "__class" in v and not prefix:
            put_record(b, etype, eid, v, prefix=k + "_")
            continue
        st = GD_TYPES.get(types.get(k, 0)) or infer(v)
        if st == "json":
            v = strip_types(v)
        if st in ("float",) and isinstance(v, int): v = float(v)
        if st == "int" and isinstance(v, float) and v.is_integer(): v = int(v)
        if st == "string" and v is None: v = ""
        b.set(etype, eid, name, v, st)


def put_curve(b, p):
    """a UCurveFloat package -> a curve entity, read the way CombatData.curve_keys / curve_extrap read it: the
    CurveFloat export's FloatCurve Keys (raw FRichCurveKey dicts) and Pre/PostInfinityExtrap (absent = RCCE_Constant)"""
    fp = R / "extract" / "json" / (p + ".json")
    fc = next((e.get("Properties", {}).get("FloatCurve", {}) for e in json.loads(fp.read_text(encoding="utf-8"))
               if e.get("Type") == "CurveFloat"), {})
    keys = fc.get("Keys", [])
    i = b.entity("curve", ent_key(p), ent_key(p), b.source("pkg", p, "extract/json/" + p + ".json"),
                 record="CombatData.curve_keys / curve_extrap")
    b.set("curve", i, "package", p, "string")
    b.set("curve", i, "times", [float(k.get("Time", 0.0)) for k in keys], "float_list")
    b.set("curve", i, "values", [float(k.get("Value", 0.0)) for k in keys], "float_list")
    b.set("curve", i, "interp", [str(k.get("InterpMode", "RCIM_Linear")) for k in keys], "string_list")
    b.set("curve", i, "keys", keys, "json")
    ex = lambda k: str(fc.get(k, "RCCE_Constant")).split("::")[-1]
    b.set("curve", i, "pre_extrap", ex("PreInfinityExtrap"), "string")
    b.set("curve", i, "post_extrap", ex("PostInfinityExtrap"), "string")
    return i


def load_opt(name):
    p = SRC / f"{name}.json"
    if not p.exists():
        print(f"  (no {p.relative_to(R)}: {name} not covered; gen_spec_src.gd -- --only={name})")
        return None
    return json.loads(p.read_text(encoding="utf-8"))


def count_nodes(n):
    if not isinstance(n, dict): return 0
    return 1 + sum(count_nodes(c.get("node") if isinstance(c, dict) and "node" in c else c)
                   for c in n.get("children", []) if isinstance(c, dict))


def strip_types(v):
    if isinstance(v, dict): return {k: strip_types(x) for k, x in v.items() if k != "__types"}
    if isinstance(v, list): return [strip_types(x) for x in v]
    return v


def snake(ue):
    """UE property name -> the record convention (bIsX -> b_is_x, HitKockbackFactor -> hit_kockback_factor)"""
    s1 = re.sub(r"(?<=[a-z0-9])([A-Z])|(?<=[A-Z])([A-Z][a-z])", r"_\1\2", ue)
    return s1.lower()


def exe_sections():
    import verify_native as vn
    vn.pinned_inputs()						# never read an exe that is not the pinned build
    f = open(vn.EXE, "rb")
    d = f.read(4096)
    pe = struct.unpack_from("<I", d, 0x3c)[0]
    ns, osz = struct.unpack_from("<H", d, pe + 6)[0], struct.unpack_from("<H", d, pe + 20)[0]
    base = struct.unpack_from("<Q", d, pe + 24 + 24)[0]
    secs = [struct.unpack_from("<IIII", d, pe + 24 + osz + 40 * i + 8) for i in range(ns)]
    def rd(addr, n):
        rva = addr - base
        for vs, va, rs, ro in secs:
            if va <= rva < va + rs:
                f.seek(ro + rva - va)
                return f.read(n)
        raise SystemExit(f"0x{addr:x} not in the exe image")
    return base, rd


def exe_enum(label):
    """UE4CodeGen FEnumeratorParam table {const char* NameUTF8; int64 Value} at the PDB label <label>::Enumerators,
    its count = FEnumParams.NumEnumerators (the int32 after the EnumeratorParams pointer in <label>::EnumParams)"""
    sys.path.insert(0, str(R / "scripts"))
    va = {}
    with open(R / "extract" / "native" / "labels_data.tsv", encoding="utf-8") as f:
        for l in f:
            c = l.rstrip("\n").split("\t")
            if len(c) > 2 and c[2] in (label + "::Enumerators", label + "::EnumParams"):
                va[c[2].rsplit("::", 1)[1]] = int(c[0], 16)
    base, rd = exe_sections()
    params = rd(va["EnumParams"], 64)
    ptrs = [struct.unpack_from("<Q", params, o)[0] for o in range(0, 56, 8)]
    o = ptrs.index(va["Enumerators"]) * 8 + 8
    n = struct.unpack_from("<i", params, o)[0]
    out = {}
    for k in range(n):
        p, v = struct.unpack("<Qq", rd(va["Enumerators"] + 16 * k, 16))
        out[v] = rd(p, 128).split(b"\0")[0].decode()
    return out


def last_arg(text, start):
    """the last top-level argument of the call whose '(' is at text[start], or None if it does not close"""
    depth, comma = 0, None
    for k in range(start, len(text)):
        ch = text[k]
        if ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0:
                return text[(comma if comma is not None else start) + 1:k].strip()
        elif ch == "," and depth == 1:
            comma = k
    return None


def has_perk_checks():
    """every HasPerk(<x>, <literal>) call of the native decomp (UPerkSystemComponent / AMordhauCharacter /
    FSkillsCustomization::HasPerk), the call joined across the lines Ghidra wraps it over"""
    hdr = re.compile(r"^// (\S.*?)\s+rva=0x([0-9a-f]+) size=")
    lit = re.compile(r"^(0x[0-9a-f]+|\d+)$")
    out = collections.defaultdict(list)
    sites = collections.defaultdict(list)	# enum value -> [{function, rva, at, code}] (every call)
    for p in sorted((R / "extract" / "native" / "decomp").glob("*.cpp")):
        cur = None
        lines = p.read_text(encoding="utf-8", errors="replace").splitlines()
        for n, l in enumerate(lines, 1):
            m = hdr.match(l)
            if m:
                cur = (m.group(1), int(m.group(2), 16))
                continue
            if "HasPerk" not in l or l.lstrip().startswith("//") or not cur:
                continue
            text = " ".join(x.strip() for x in lines[n - 1:n + 5])
            for c in re.finditer(r"HasPerk\s*\(", text):
                if c.start() >= len(l.strip()) + 1:
                    break				# a call that starts on a later line is found from that line
                a = last_arg(text, c.end() - 1)
                if a is None or not lit.match(a):
                    continue
                v = int(a, 0)
                key = f"{cur[0]} rva=0x{cur[1]:x}"
                if key not in out[v]:
                    out[v].append(key)
                sites[v].append({"function": cur[0], "rva": f"0x{cur[1]:x}",
                                 "at": f"extract/native/decomp/{p.name}:{n}", "code": text[max(0, c.start() - 60):c.start() + 140]})
    has_perk_checks.sites = sites
    return out


def ent_key(path):
    return pathlib.PurePosixPath(path).name


def records(b, ctx=None):
    """entity adapter: every record dump (godot/data_gen/spec_src) -> Entities + T_ values"""
    rec_src = {t: [b.source("record", f) for f in files] for t, (_, files) in TYPES.items()}
    b.source("dump", "godot/tools/gen_spec_src.gd", "dumps the components/ue records to godot/data_gen/spec_src")
    # ---- weapons + attacks -------------------------------------------------------------------------------------
    weapons = load("weapons")
    motion_ids = {}
    motions = load("motions")
    for key in sorted(motions):
        rec = motions[key]
        if key.startswith("native:"):
            eid = b.entity("motion", "NATIVE_" + key[7:], key[7:], b.source("native", key[7:], "NativeCtor replay"), record=rec["__class"])
        else:
            eid = b.entity("motion", ent_key(key), ent_key(key), b.source("pkg", key, "extract/json/" + key + ".json"),
                           record=rec["__class"])
        motion_ids[key] = eid
        put_record(b, "motion", eid, rec)
    for wid in sorted(weapons):
        e = weapons[wid]
        w = e["weapon"]
        weid = b.entity("weapon", wid, w.get("display_name", wid), b.source("pkg", w["class_path"], "extract/json/" + w["class_path"] + ".json"),
                        record="WeaponData")
        put_record(b, "weapon", weid, w, skip=set(ATTACK_SLOTS) | {"unset"})
        b.set("weapon", weid, "native_default_ue_fields", list(w.get("unset", [])), "string_list")
        if e.get("equip"):
            put_record(b, "weapon", weid, e["equip"], skip={"path", "native", "b_has_alternate_mode", "b_is_two_handed",
                                                             "b_second_is_two_handed"}, prefix="equip_")
        b.set("weapon", weid, "profile", e.get("profile", ""), "string")
        b.set("weapon", weid, "parry_motion", motion_ids.get(e.get("parry_motion", ""), ""), "ref", "motion")
        for mk in ("motions", "alt_motions"):
            for mv, bp in sorted(e.get(mk, {}).items()):
                b.set("weapon", weid, ("alt_" if mk == "alt_motions" else "") + "motion_" + mv.lower(), motion_ids[bp], "ref", "motion")
        if "alt_profile" in e:
            b.set("weapon", weid, "alt_profile", e["alt_profile"], "string")
            b.set("weapon", weid, "alt_parry_motion", motion_ids.get(e.get("alt_parry_motion", ""), ""), "ref", "motion")
        for slot in ATTACK_SLOTS:
            a = w.get(slot)
            if a is None:
                continue
            aid = b.entity("attack", f"{wid}_{slot.upper()}", f"{wid} {slot}", b.entities[weid]["source_id"], parent=weid,
                           record="AttackInfo")
            put_record(b, "attack", aid, a, skip={"unset"})
            b.set("attack", aid, "native_default_ue_fields", list(a.get("unset", [])), "string_list")
            b.set("weapon", weid, "attack_" + slot, aid, "ref", "attack")
    # ---- character / movement / stats / camera / perks ----------------------------------------------------------
    ch = load("character")
    char_src = b.source("pkg", "Mordhau/Content/Mordhau/Blueprints/Characters/BP_MordhauCharacter",
                        "extract/json/Mordhau/Content/Mordhau/Blueprints/Characters/BP_MordhauCharacter.json")
    eid = b.entity("character", "BP_MordhauCharacter", "BP_MordhauCharacter", char_src, record="CharacterData.Character")
    put_record(b, "character", eid, ch["character"])
    # block-collider terms UpdateBlockCollider rva=0x15700a0 reads (merged native ctor + CDO; FTransforms as UE dicts)
    raw = ch.get("character_raw", {})
    if "BlockColliderForwardParryDistance" in raw:
        v = raw["BlockColliderForwardParryDistance"]
        b.set("character", eid, "block_collider_forward_parry_distance", [v["X"], v["Y"]], "vec2")
    for k in ("LowBlockColliderRelativeOffset", "HighBlockColliderRelativeOffset"):
        if k in raw:
            b.set("character", eid, snake(k), raw[k], "json")
    b.set("character", eid, "kick_weapon", sid("ENT_WPN", ent_key(ch["kick_weapon"])), "ref", "weapon")
    for k, et, rec in (("movement", "movement", "CharacterData.Movement"), ("move_extra", "movement", "CharacterData.MoveExtra"),
                       ("camera", "camera", "CharacterData.Camera"), ("stamina", "stat", "CharacterData.Stat"),
                       ("health", "stat", "CharacterData.Stat")):
        i = b.entity(et, k.upper(), k, char_src, record=rec)
        put_record(b, et, i, ch[k])
    if "physics" in ch:		# MordhauMovement.load_default: [/Script/Engine.PhysicsSettings] of DefaultEngine.ini
        i = b.entity("physics", "WORLD", "PhysicsSettings", b.source("ini", "DefaultEngine.ini [/Script/Engine.PhysicsSettings]",
                     "MordhauMovement.load_data (UeConfig)"), record="MordhauMovement gravity_z / terminal_velocity")
        for k, v in sorted(ch["physics"].items()):
            b.set("physics", i, k, float(v), "float")
    # UCurveFloat curves the rules sample: the movement turn-sprint curves and every capture point's capture /
    # neutralize speed curve (FL control points). Read the way CombatData.curve_keys / curve_extrap read them: the
    # CurveFloat export's FloatCurve Keys (raw FRichCurveKey dicts: InterpMode, TangentWeightMode, Time, Value,
    # ArriveTangent, LeaveTangent) and Pre/PostInfinityExtrap (absent = RCCE_Constant, the reader's default)
    cpaths = set(ch.get("curves", {}))
    # the Horde curves the mode rules sample (golden/mode.gd: damage by player count, Demon Invasion scaling / JIP coins)
    cpaths |= {"Mordhau/Content/Mordhau/Blueprints/GameModes/Horde/FC_HordeDamageModifier",
               "Mordhau/Content/Mordhau/Blueprints/GameModes/Horde/DemonInvasion/Data/FC_DemonHordeDifficultyScaling",
               "Mordhau/Content/Mordhau/Blueprints/GameModes/Horde/DemonInvasion/Data/FC_DemonHordeJIPCoins"}
    cpaths = {p for p in cpaths if (R / "extract" / "json" / (p + ".json")).exists()}
    for m in load("modes").values():
        for cp in m["data"].get("control_points", []) or []:
            for k in ("capture_speed_curve", "neutralize_speed_curve"):
                if cp.get(k):
                    cpaths.add(cp[k])
    for p in sorted(cpaths):
        put_curve(b, p)
    pk = load("perks")
    enum = exe_enum("Z_Construct_UEnum_Mordhau_EPerk")		# value -> "EPerk::Name", from the exe's reflection table
    by_name = {v.split("::")[-1]: k for k, v in enum.items()}
    checks = has_perk_checks()								# enum value -> ["Name rva=0x.."] (native decomp)
    effects = {k: v for k, v in pk.get("perk_effects", {}).items()}
    esrc = b.source("rdata", "extract/native/labels_data.tsv:Z_Construct_UEnum_Mordhau_EPerk::Enumerators",
                    "EPerk FEnumeratorParam table read from the hash-pinned exe")
    for p in pk["perks"]:
        i = b.entity("perk", ent_key(p["path"]), p["name"], b.source("pkg", p["path"], "extract/json/" + p["path"] + ".json"), record="UeWearable.perks")
        for k in ("cost", "enum", "name", "path"):
            b.set("perk", i, k, p[k], infer(p[k]))
        # a CDO that never writes Enum keeps 0 (UPerk ctor rva=0x16488f0 leaves it, ue_wearable.gd perks())
        ev = by_name[p["enum"].split("::")[-1]] if "::" in p["enum"] else int(p["enum"])
        b.set("perk", i, "enum_value", ev, "int")
        b.set("perk", i, "enum_name", enum[ev], "string")
        b.set("perk", i, "checked_by", checks.get(ev, []), "string_list")
        stem = enum[ev].split("::")[-1]
        info = {"entity": i, "value": ev, "enum": enum[ev], "stem": stem, "params": []}
        for k in sorted(effects):
            if k.startswith(stem) and k[len(stem):len(stem) + 1].isupper():
                v = effects.pop(k)
                b.set("perk", i, "effect_" + snake(k), v, infer(v))
                info["params"].append(k)
        b.perk_info.append(info)
    i = b.entity("perk", "PERK_SYSTEM", "UPerkSystemComponent", b.source("native", "UPerkSystemComponent", "NativeCtor replay"),
                 record="CharacterData.Perks")
    put_record(b, "perk", i, pk["perk_system"])
    b.set("perk", i, "character_points", pk["character_points"], "int")
    for k in sorted(effects):		# UPerkSystemComponent parameters no single perk's name prefixes
        b.set("perk", i, "effect_" + snake(k), effects[k], infer(effects[k]))
    # ---- modes + maps ------------------------------------------------------------------------------------------
    modes = load("modes")
    map_users = collections.defaultdict(list)
    for mid in sorted(modes):
        m = modes[mid]
        d = m["data"]
        i = b.entity("mode", mid, mid, b.source("pkg", d["game_mode"], "extract/json/" + d["game_mode"] + ".json"),
                     record=d["__class"])
        put_record(b, "mode", i, d)
        put_record(b, "mode", i, m["rules"], prefix="rules_")
        for k, v in sorted(m.get("table", {}).items()):
            b.set("mode", i, "table_" + k, v, infer(v))
            if k in ("map", "fallback") and v:
                map_users[v].append(mid)
        b.set("mode", i, "metadata", m.get("metadata", ""), "string")
    # Horde + Battle Royale (gen_spec_src horde: the golden mode exporter's own readers; rust-mode-ai request r10)
    hz = load_opt("horde")
    if hz:
        for mid, key, gm in (("HRD", "hrd_mode", "Mordhau/Content/Mordhau/Blueprints/GameModes/Horde/BP_HordeGameMode"),
                             ("BR", "br_mode", "Mordhau/Content/Mordhau/Blueprints/GameModes/BattleRoyale/BP_BattleRoyaleGameMode")):
            i = b.entity("mode", mid, mid, b.source("pkg", gm, "extract/json/" + gm + ".json"), record="MordhauModeData")
            put_record(b, "mode", i, hz[key])
            rk = mid.lower() + "_rules"		# CombatData.mode_rules, like the six modes' rules_* (gen_spec_src horde)
            if rk in hz:
                put_record(b, "mode", i, hz[rk], prefix="rules_")
        b.set("mode", "ENT_MODE_BR", "round_start_duration", float(hz["br"]["round_start_duration"]), "float")
        h = hz["horde"]
        i = b.entity("horde", "HORDE", "Horde", b.source("record", "godot/tools/golden/mode.gd",
                     "_horde_data / _horde_extras (the reference's Horde readers)"), record="golden/mode.gd _horde_data")
        for k, v in sorted(h["config"].items()):
            b.set("horde", i, "config_" + snake(k), v, "json" if isinstance(v, (dict, list)) or v is None else infer(v))
        for k in ("squad_waves", "squads", "enemies"):
            b.set("horde", i, k, h[k], "json")
        b.set("horde", i, "damage_by_player_count", [float(x) for x in h["damage_by_player_count"]], "float_list")
        b.set("horde", i, "damage_curve", str(h.get("damage_curve", "")), "string")
        for k, v in sorted(h["extras"].items()):
            b.set("horde", i, "extras_" + k, v, "json")
        names = sorted(hz["horde_skill_names"].items(), key=lambda kv: kv[1])
        b.set("horde", i, "skill_enum_names", [n.split("::")[-1] for n, _ in names], "string_list")
        b.set("horde", i, "skill_enum_values", [int(v) for _, v in names], "int_list")
    maps_dir = R / "godot" / "data_gen" / "maps"
    scenes = {p.stem: p for p in sorted(maps_dir.glob("*.tscn"))} if maps_dir.exists() else {}
    actors = load_opt("maps") or {}
    nav = actors.pop("__nav_agent", None)
    for nm in sorted(set(scenes) | set(actors)):
        a = actors.get(nm)
        src = b.source("pkg", a["package"], "extract/json/" + a["package"] + ".umap.json (+ streamed sub-levels)") if a else \
            b.source("generated", "godot/data_gen/maps/" + scenes[nm].name, "godot/tools/gen_level.gd output")
        i = b.entity("map", nm, nm, src, record="ModeData.player_starts/control_points/nav_bounds" if a else "level scene")
        b.set("map", i, "mode_prefix", nm.split("_", 1)[0], "string")
        if nm in scenes:
            res = "res://data_gen/maps/" + scenes[nm].name
            b.set("map", i, "scene", res, "string")
            b.set("map", i, "used_by_modes", [sid("ENT_MODE", x) for x in sorted(set(map_users.get(res, [])))], "string_list")
        if a:
            b.set("map", i, "package", a["package"], "string")
            for k in ("player_starts", "control_points", "nav_bounds"):
                b.set("map", i, k, a[k], "json")
                b.set("map", i, "n_" + k, len(a[k]), "int")
    if nav:
        i = b.entity("nav", "AGENT", "Recast nav agent", b.source("ini", "DefaultEngine.ini /Script/NavigationSystem.RecastNavMesh",
                     "ModeData.nav_agent (DefaultEngine.ini over BaseEngine.ini)"), record="ModeData.NavAgentDef")
        put_record(b, "nav", i, nav)
    # ---- AI behaviour trees ----------------------------------------------------------------------------------------
    for nm, t in sorted((load_opt("ai_trees") or {}).items()):
        i = b.entity("bt", nm, nm, b.source("pkg", t.get("asset", nm), "extract/json/" + t.get("asset", nm) + ".json"),
                     record="BotData.TreeDef")
        b.set("bt", i, "blackboard", t.get("blackboard", {}), "json")
        b.set("bt", i, "root", strip_types(t.get("root")), "json")
        b.set("bt", i, "n_nodes", count_nodes(t.get("root")), "int")
    # ---- user settings defaults (UMordhauGameUserSettings::SetToDefaults + CVar defaults) --------------------------
    st = load_opt("settings")
    if st:
        ssrc = b.source("record", "godot/game/ui/user_settings.gd", "UserSettings: SetToDefaults rva=0x15a7c90 + CVar initializers")
        for kind, d in (("field", st["v"]), ("cvar", st["cvars"])):
            for k, v in sorted(d.items()):
                i = b.entity("setting", f"{kind.upper()}_{k}", k, ssrc, record="UserSettings." + ("v" if kind == "field" else "cvars"))
                b.set("setting", i, "kind", kind, "string")
                b.set("setting", i, "default", v, "json")
        for k in ("version", "mordhau_version"):
            i = b.entity("setting", "FIELD_" + k, k, ssrc, record="UserSettings")
            b.set("setting", i, "kind", "field", "string")
            b.set("setting", i, "default", st[k], "json")
    # ---- loadout / customization rules ----------------------------------------------------------------------------
    cu = load_opt("customization")
    if cu:
        csrc = b.source("record", "godot/components/ue/records/customization_data.gd", "CustomizationData (BP_MordhauSingleton Equipment)")
        for k, r in sorted(cu["equipment"].items(), key=lambda kv: int(kv[0])):
            i = b.entity("equipment", f"{int(k):03d}", ent_key(r.get("path", "")) or k, csrc, record="CustomizationData.EquipmentRules")
            put_record(b, "equipment", i, r)
            w = sid("ENT_WPN", ent_key(r.get("path", "")))
            b.set("equipment", i, "weapon", w if w in b.entities else None, "ref", "weapon")
        perks = sorted((e for e in b.entities.values() if e["entity_type"] == "perk" and e["entity_id"] != "ENT_PERK_PERK_SYSTEM"),
                       key=lambda e: e["entity_id"])
        b.set("perk", "ENT_PERK_PERK_SYSTEM", "perk_costs_by_bit", cu["perk_costs"], "json")
    # ---- animation assets the motions / weapons use ------------------------------------------------------------------
    for p, a in sorted((load_opt("anims") or {}).items()):
        i = b.entity("anim", ent_key(p), ent_key(p), b.source("pkg", p, "extract/json/" + p + ".json"), record="AnimData")
        b.set("anim", i, "kind", a["kind"], "string")
        b.set("anim", i, "package", p, "string")
        if "length" in a:
            b.set("anim", i, "length", a["length"], "float")
            b.set("anim", i, "additive", a["additive"], "bool")
        if "montage" in a:
            m = a["montage"]
            for k in ("length", "slot", "blend_in_time", "blend_in_option", "blend_out_time", "blend_out_option", "blend_out_trigger_time"):
                b.set("anim", i, "montage_" + k, m[k], "float" if isinstance(m[k], (int, float)) else "string")
            b.set("anim", i, "montage_segments", strip_types(m.get("segments", [])), "json")
        b.set("anim", i, "notifies", a.get("notifies", []), "json")
        b.set("anim", i, "n_notifies", len(a.get("notifies", [])), "int")
    # ---- bots + wearables ----------------------------------------------------------------------------------------
    for nm, rec in sorted(load("bots").items()):
        p = "Mordhau/Content/Mordhau/Blueprints/BotProfiles/BotBehaviorProfiles/" + nm
        i = b.entity("bot", nm, nm, b.source("pkg", p, "extract/json/" + p + ".json"), record="BotData.Profile")
        put_record(b, "bot", i, rec)
    wear = load("wearables")
    for wid in sorted(wear):
        w = wear[wid]
        i = b.entity("wearable", wid, w.get("display_name", wid), b.source("pkg", w["class_path"], "extract/json/" + w["class_path"] + ".json"),
                     record="WearableData")
        put_record(b, "wearable", i, w)
    del wear
    # ---- constants: .rdata literals + Blueprint bytecode literals ----------------------------------------------
    rdata = {}
    with open(R / "extract" / "native" / "rdata.tsv", encoding="utf-8") as f:
        next(f)
        for l in f:
            c = l.rstrip("\n").split("\t")
            rdata[int(c[0], 16)] = (c[4], c[5])
    rsrc = b.source("rdata", "extract/native/rdata.tsv", "hash-pinned exe .rdata (scripts/rdata_consts.py)")
    krx = re.compile(r'(_k|_k64)\("(\w+)",\s*0x([0-9a-fA-F]+),\s*(\[[^\]]*\]|\w+)', re.S)
    for gd in sorted((R / "godot" / "components" / "ue" / "rdata").glob("*.gd")):
        txt = gd.read_text(encoding="utf-8")
        consts = dict(re.findall(r'const (_\w+) := "([^"]*)"', txt))
        group = gd.stem.replace("_constants", "").upper()
        for m in krx.finditer(txt):
            va = int(m.group(3), 16)
            fns = [consts.get(x.strip(), x.strip().strip('"')) for x in m.group(4).strip("[]").replace("\n", " ").split(",") if x.strip()]
            raw = rdata.get(va)
            if raw is None:
                raise SystemExit(f"{gd.name}: {m.group(2)} va 0x{va:x} not in rdata.tsv")
            line = txt.count("\n", 0, m.start()) + 1
            i = b.entity("constant", f"{group}_{m.group(2)}", m.group(2), rsrc, record=f"{gd.relative_to(R).as_posix()}:{line}")
            if m.group(1) == "_k64":	# an f64 literal: kept as a double (decimal text, exact)
                b.set("constant", i, "value_f64", float(raw[1]), "double")
            else:
                b.set("constant", i, "value", float(raw[0]), "float")
            b.set("constant", i, "va", f"0x{va:x}", "string")
            b.set("constant", i, "width", "f64" if m.group(1) == "_k64" else "f32", "string")
            b.set("constant", i, "functions", fns, "string_list")
            b.set("constant", i, "group", group, "string")
    # engine literals the port reads straight from rdata.tsv rather than through an rdata/*.gd constant
    # (CombatData.curve_eval: the FRichCurve Bezier 1/3, _DAT_14406a414, loaded by FRichCurve::EvalForTwoKeys
    # rva=0x3024fa0, engine code compiled into the exe; mh-sim reads the same va)
    for name, va, fns in (("curve_third", 0x14406a414, ["FRichCurve::EvalForTwoKeys"]),):
        i = b.entity("constant", f"ENGINE_{name}", name, rsrc, record="godot/components/ue/records/combat_data.gd curve_eval")
        b.set("constant", i, "value", float(rdata[va][0]), "float")
        b.set("constant", i, "va", f"0x{va:x}", "string")
        b.set("constant", i, "width", "f32", "string")
        b.set("constant", i, "functions", fns, "string_list")
        b.set("constant", i, "group", "ENGINE", "string")
    kp = R / "godot" / "data_gen" / "mode" / "mode_kismet.json"
    ksrc = b.source("kismet", "godot/data_gen/mode/mode_kismet.json", "scripts/mode_kismet.py (Blueprint bytecode literals)")
    for k, v in sorted(json.loads(kp.read_text(encoding="utf-8")).items()):
        i = b.entity("constant", "KISMET_" + k, k, ksrc, record="ModeKismet")
        b.set("constant", i, "literal", v["value"], "json")	# typed as bytecode gives it: int / float / str / list
        b.set("constant", i, "bp_src", v["src"], "string")
        b.set("constant", i, "group", "KISMET", "string")
    import sheets_extra				# Horde / BR, cosmetics, audio (sheets r5)
    sheets_extra.run(sys.modules[__name__], b)


def populate():
    """this game's populate as a spec-kit Pipeline (tools/spec-kit/speckit/adapters.py): the worked example"""
    import sheets_tiers				# evidence tiers a/b/c/d per field and rule (sheets r6)
    import sheets_character_exe		# mh-character exe records via mh-host (sheets r10)
    me = sys.modules[__name__]
    one = lambda f: (lambda b, ctx: f(b))
    return Pipeline([
        FnAdapter("entity", records, "records"),
        FnAdapter("entity", lambda b, ctx: sheets_character_exe.run(me, b, ctx), "character_exe"),  # horses / projectiles / ladder (r10)
        FnAdapter("field", one(describe_fields), "describe_fields"),
        FnAdapter("field", one(assign_units), "assign_units"),
        FnAdapter("rule", one(rules), "rules"),
        FnAdapter("evidence", one(evidence), "evidence"),
        FnAdapter("finish", lambda b, ctx: sheets_tiers.compute(me, b), "tiers"),
    ]).populate(Book())


def describe_fields(b):
    """Fields.source / description: the record file line declaring `var <name>` (its comment carries the ctor offset cite)"""
    cache = {}
    for f in b.fields.values():
        nm = f["name"]
        base = nm.split("_", 1)[1] if nm.startswith(("equip_", "rules_", "table_")) else nm
        for path in TYPES[f["entity_type"]][1]:
            if path not in cache:
                cache[path] = (R / path).read_text(encoding="utf-8").splitlines() if (R / path).exists() else []
            for n, l in enumerate(cache[path], 1):
                if path.endswith(".rs"):		# a Rust record struct: `pub <name>: T, // cite`
                    if re.match(rf"\s*pub {re.escape(base)}\s*:", l):
                        f["source"] = f"{path}:{n}"
                        f["description"] = l.split("//", 1)[1].strip()[:300] if "//" in l else ""
                        break
                elif re.match(rf"\s*(@export\s+)?var {re.escape(base)}\b", l):
                    f["source"] = f"{path}:{n}"
                    f["description"] = l.split("#", 1)[1].strip()[:300] if "#" in l else ""
                    break
            if f["source"]:
                break
        if not f["source"]:
            xr = "core/crates/mh-host/src/exe_records.rs dump() (mh-character's loaders)"
            f["source"] = {"constant": "extract/native/rdata.tsv / mode_kismet.json", "map": "godot/data_gen/maps",
                           "horse": xr, "projectile": xr, "ladder": xr, "equipment_movement": xr}.get(
                f["entity_type"], "godot/tools/gen_spec_src.gd (derived)")
        if nm == "native_default_ue_fields":
            f["description"] = "UE property names the Blueprint chain never writes: they hold the native constructor value"


# ---- units ------------------------------------------------------------------------------------------------------
# A unit is set only with a source (unit_source says which kind):
#   stated:     a record header / comment states it (UNITS above)
#   use:        the port's rule code uses the field that way, cited file:line (added to / compared with motion time ->
#               s; passed to OffsetStamina -> stamina; turned into radians -> deg; compared with a distance -> cm)
#   convention: UE units (DESIGN §4: centimetres, seconds) for the UE property the name is (MaxWalkSpeed, ...), or a
#               dimensionless name (Factor / Modifier / Multiplier / Probability); weaker than use, labelled as such
#   type:       not a quantity (bool, text, reference, JSON struct): "-"
# Anything else stays "" + UNCONFIRMED.
CODE_TIME = re.compile(r"\b(start_time|end_time|now|time|dt|real_time|elapsed|world_time|_now|t0)\b|_time\b|_end\b")
CODE_OP = re.compile(r"[-+<>]")
CONVENTION = [
    (re.compile(r"(^|_)probability($|_)"), "probability [0..1]", "dimensionless: a probability (name)"),
    (re.compile(r"(factor|modifier|multiplier|scale|exponent|percentage|play_rate|anim_rate|_rate_clamp|normalized\w*)$|_factor_|_modifier_|normalized|exponent"),
     "ratio", "dimensionless: a factor / modifier / rate multiplier / normalized value (name)"),
    (re.compile(r"(^|_)(movement_restriction)$|restriction$"), "enum EMovementRestriction", "enum value (CombatEnums.MovementRestriction, PDB LF_ENUM)"),
    (re.compile(r"^(initial_value|max_value|min_value)$"), "stat points", "the stat's own value: HP for HEALTH, stamina points for STAMINA"),
    (re.compile(r"^(max_walk_speed|max_walk_speed_crouched|max_walk_speed_crouched_with_rat_perk|max_speed_falling|jump_z_velocity|"
                r"min_velocity_for_fall_damage|ragdoll_min_velocity_for_fall_damage|air_control_boost_velocity_threshold|"
                r"ragdoll_falling_min_velocity_to_get_up|knockback_up_impulse)$"), "cm/s",
     "UE velocity in cm/s (DESIGN §4: UE units are centimetres)"),
    (re.compile(r"^(max_acceleration|walk_acceleration|sprint_acceleration|partial_sprint_acceleration|braking_deceleration_falling|"
                r"braking_deceleration_falling_too_fast)$"), "cm/s^2", "UE acceleration in cm/s^2 (DESIGN §4)"),
    (re.compile(r"(turn_cap|turn_rate|look_up_rate)$"), "deg/s", "UE rotation rate in degrees/s (FRotator degrees)"),
    (re.compile(r"(look_up_limit|look_down_limit|angle_tolerance|angling_limits|field_of_view|fov_offset|_angle|rotation_offset)$"),
     "deg", "UE angles are degrees (FRotator)"),
    (re.compile(r"^(max_step_height|crouched_half_height|slide_radius|length|second_length|right_hand_equip_offset|"
                r"third_person_camera_offset|grip_location_local)$"), "cm", "UE distance in cm (DESIGN §4)"),
    (re.compile(r"^(radius|height|max_step|cell_size|cell_height)$|mesh_z_offset"), "cm",
     "UE distance in cm (DESIGN §4; nav agent: Recast AgentRadius / AgentHeight / AgentMaxStepHeight / CellSize / CellHeight)"),
    (re.compile(r"^max_slope$"), "deg", "Recast AgentMaxSlope is degrees"),
    (re.compile(r"^(length|montage_length)$"), "s", "UAnimSequenceBase SequenceLength: clip time in seconds"),
    (re.compile(r"^n_|^enum_value$"), "count", "a count / enum value the spec derives (name)"),
    (re.compile(r"stamina_(drain|cost|negation|clamp|reward|recover|on_hit|on_kill)|_stamina_drain$|stamina_on_(hit|kill)$"),
     "stamina", "stamina points (name: a StaminaStatComponent amount)"),
    (re.compile(r"_damage$|damage_max$"), "hp", "damage points (name)"),
    (re.compile(r"(point_cost|^cost|character_points)$"), "character points", "loadout point budget (CustomizationData)"),
    (re.compile(r"(_mask)$"), "bitmask", "bit flags (name)"),
    (re.compile(r"(_team|^team|_winner|armor_class|armor_tier_override|^id)$"), "index", "an index / enum value (name)"),
    (re.compile(r"(count|rounds_to_win|team_count|max_people_per_room|starting_tickets|ticket_cost|drain_amount|coins_per_\w+|"
                r"max_loss_streak|win_condition_rounds|score_change|score_to_win)$"), "count", "a count (name)"),
    (re.compile(r"(time|duration|window|delay|cooldown|interval|recovery|blend_in|blend_out|lock_out|lockout|_windup|fade|_extra)"),
     "s", "UE time is seconds (UWorld::GetTimeSeconds); the name is a duration"),
]
STAMINA_RX = re.compile(r"offset_stamina\(|\bstamina\b")
ANGLE_RX = re.compile(r"deg_to_rad\(|rad_to_deg\(|_deg\b")
DIST_RX = re.compile(r"\b(dist|distance|length|radius)\b")
HP_RX = re.compile(r"\b(health|apply_damage|damage)\b")


def code_uses():
    """field name -> [(file:line, code)] of every `.name` read in godot/game, plus one hop through a local it is
    copied into (`var per := character.stamina_regen_per_tick` -> the later lines using `per`, same function)"""
    idx = collections.defaultdict(list)
    for p in sorted((R / "godot" / "game").rglob("*.gd")):
        rel = p.relative_to(R).as_posix()
        lines = [l.split("#", 1)[0] for l in p.read_text(encoding="utf-8").splitlines()]
        for n, c in enumerate(lines, 1):
            for m in re.finditer(r"\.([a-z_][a-z0-9_]*)\b", c):
                idx[m.group(1)].append((f"{rel}:{n}", c))
            a = re.match(r"\s*(?:var\s+)?([a-z_]\w*)\s*:?=\s*[\w.]*\.([a-z_]\w*)\s*$", c)
            if a:
                alias, field = a.groups()
                for k in range(n, min(n + 40, len(lines))):
                    if re.match(r"(static )?func ", lines[k]): break
                    if re.search(rf"\b{alias}\b", lines[k]):
                        idx[field].append((f"{rel}:{k + 1} (via {alias} = .{field} at line {n})", lines[k].replace(alias, "." + field)))
    return idx


def assign_units(b):
    uses = code_uses()
    numeric = {"int", "float", "double", "vec2", "vec3", "float_list", "int_list"}
    for f in b.fields.values():
        if f["unit"]:
            f["unit_source"] = "stated: " + f["unit_source"]
            continue
        nm = f["name"]
        base = nm.split("_", 1)[1] if nm.startswith(("equip_", "rules_", "table_", "scoring_", "state_")) else nm
        if f["type"] not in numeric:
            f["unit"], f["unit_source"] = "-", f"type: not a quantity ({f['type']})"
            continue
        if f["entity_type"] == "constant":
            f["unit"], f["unit_source"] = ("address", "stated: rdata.tsv va") if nm == "va" else \
                ("per entity", "stated: each constant's unit is in the comment above its _k() line (entity record = file:line)")
            continue
        hit = None
        # name rules that pin a non-time unit (every rule but the last, the duration-name one) win over a time use: a
        # velocity compared with a velocity delta, an exponent clamp next to a time, an id are not durations
        for rx, u, why in CONVENTION[:-1]:
            if rx.search(base) and not (u == "cm" and f["entity_type"] == "anim"):	# an anim's length is its clip time
                hit = (u, "convention: " + why); break
        for at, code in ([] if hit or f["entity_type"] == "stat" else uses.get(base, [])):
            rest = code.replace("." + base, " ")
            if "offset_stamina(" in code or re.search(r"\bstamina\s*[-+]?=|<\s*stamina|stamina\s*<", rest):
                hit = ("stamina", f"use: {at} offsets / compares stamina with it"); break
            if re.search(rf"(deg_to_rad|rad_to_deg)\(\s*[\w.]*\.{base}\s*\)", code):
                hit = ("deg", f"use: {at} converts it to radians / compares degrees"); break
            if re.search(r"\b(health)\b", rest) and CODE_OP.search(code):
                hit = ("hp", f"use: {at} offsets / compares health with it"); break
            if CODE_OP.search(code) and CODE_TIME.search(rest) and not DIST_RX.search(rest):
                hit = ("s", f"use: {at} adds it to / compares it with a time"); break
        if hit is None:
            for rx, u, why in CONVENTION:
                if rx.search(base):
                    hit = (u, "convention: " + why); break
        if hit:
            f["unit"], f["unit_source"] = hit
        else:
            f["unit_source"] = "UNCONFIRMED: no source states a unit and no rule uses it as one"


# ---- rules -------------------------------------------------------------------------------------------------------
CITE = re.compile(r"([A-Za-z_]\w*(?:::[A-Za-z_~]\w*)+) rva=0x([0-9a-f]+)")
CURATED = [
    # id, type, description, PDB names composed (each must be cited by the port), params (from spec_src/rules.json key)
    ("RULE_ATTACK_SLOT_BY_MOVE", "lookup", "EAttackMove -> the weapon's FAttackInfo slot the attack reads",
     ["AMordhauWeapon::GetBaseAttackInfo"], "attack_slot_by_move", ["FLD_WPN_ATTACK_STRIKE", "FLD_WPN_ATTACK_STAB",
                                                                    "FLD_WPN_ATTACK_KICK", "FLD_WPN_ATTACK_COUCH", "FLD_WPN_ATTACK_BASH"]),
    ("RULE_ALT_MODE_ATTACK_SWAP", "swap", "alternate weapon mode swaps each pair of weapon fields",
     ["AMordhauWeapon::OnRequestModeSwitch_Implementation"], "alt_mode_swaps", []),
    ("RULE_ATTACK_TIMELINE", "timeline", "windup_end = start + Windup' ; release_end = windup_end + Release' ; "
     "end = release_end + MissRecovery' (primes: ModifyAttackInfo by attack type, character modifiers, lag)",
     ["UAttackMotion::OnBegin_Implementation", "UAttackMotion::ModifyAttackInfo_Implementation",
      "UAttackMotion::ComputeWindup_Implementation"], None,
     ["FLD_ATK_WINDUP", "FLD_ATK_RELEASE", "FLD_ATK_MISS_RECOVERY", "FLD_ATK_COMBO_WINDUP_INCREASE",
      "FLD_ATK_MISS_COMBO_EXTRA_WINDUP_INCREASE", "FLD_MOT_MORPH_WINDUP_MODIFIER", "FLD_MOT_RIPOSTE_WINDUP_MODIFIER"]),
    ("RULE_ATTACK_WINDOWS", "window", "feint / morph / chamber / recovery-queue windows of an attack motion",
     ["UAttackMotion::OnTick_Implementation", "UAttackMotion::ProcessFeint_Implementation", "UAttackMotion::CanMorphInto"],
     None, ["FLD_MOT_FEINT_WINDOW", "FLD_MOT_MORPH_WINDOW", "FLD_MOT_CHAMBER_WINDOW", "FLD_MOT_RECOVERY_QUEUE_WINDOW",
            "FLD_MOT_MIN_WINDUP_TIME_BEFORE_MORPHING", "FLD_MOT_MAX_MORPH_TOTAL_TIME", "FLD_MOT_HIT_RECOVERY"]),
    ("RULE_FEINT", "stamina_cost", "a feint pays FeintCost (ChamberFeintCost after a chamber) and locks out for FeintLockOut",
     ["UFeintedMotion::OnBegin_Implementation", "UAttackMotion::ProcessFeint_Implementation"], None,
     ["FLD_ATK_FEINT_COST", "FLD_ATK_CHAMBER_FEINT_COST", "FLD_ATK_FEINT_LOCK_OUT"]),
    ("RULE_PARRY", "timeline", "parry window and recoveries (success / fail / miss) of the parry motion",
     ["UParryMotion::OnBegin_Implementation"], None, ["FLD_WPN_PARRY_WINDOW_OFFSET", "FLD_WPN_B_IS_PARRY_HELD"]),
    ("RULE_BLOCKED", "timeline", "an attack blocked by a parry or the world: bounce and recovery",
     ["UBlockedMotion::OnBegin_Implementation"], None, ["FLD_MOT_CLASHED_RECOVERY", "FLD_MOT_HIT_STOP_RECOVERY"]),
    ("RULE_FLINCH", "timeline", "flinch on hit", ["UFlinchMotion::OnBegin_Implementation"], None,
     ["FLD_ATK_FLINCH_DURATION_MODIFIER", "FLD_ATK_FLINCH_SPEED_MODIFIER"]),
    ("RULE_STAMINA", "stat", "stamina offset, regeneration tick and stop",
     ["UStaminaStatComponent::OffsetStamina", "UStaminaStatComponent::TickStat", "UStatComponent::StopRegeneration"], None,
     ["FLD_ATK_STAMINA_DRAIN", "FLD_ATK_MISS_STAMINA_COST", "FLD_ATK_HIT_STAMINA_REWARD", "FLD_WPN_BLOCK_STAMINA_NEGATION"]),
    ("RULE_DAMAGE_MODIFIERS", "modifier", "damage by armor tier + head / leg bonus, then damage modifiers",
     ["UDamageableComponent::ModifyDamage"], None, ["FLD_ATK_DAMAGE", "FLD_ATK_HEAD_BONUS", "FLD_ATK_LEG_BONUS"]),
    ("RULE_MOVEMENT_RESTRICTION", "restriction", "the movement restriction a motion / weapon imposes",
     ["AMordhauCharacter::GetMovementRestriction", "UAttackMotion::GetMovementRestriction",
      "UParryMotion::GetMovementRestriction", "UKickMotion::GetMovementRestriction"], None,
     ["FLD_MOT_MOVEMENT_RESTRICTION", "FLD_WPN_BLOCK_MOVEMENT_RESTRICTION"]),
    ("RULE_CAPTURE", "capture", "capture point progress (per-pawn score intervals, presence)",
     ["AControlPoint::Tick", "AControlPoint::UpdateCaptureProgress"], None, []),
]


def rules(b):
    cites = collections.defaultdict(lambda: {"names": set(), "at": []})
    files = sorted((R / "godot" / "game").rglob("*.gd")) + sorted((R / "godot" / "components" / "ue" / "records").glob("*.gd"))
    for p in files:
        rel = p.relative_to(R).as_posix()
        for n, l in enumerate(p.read_text(encoding="utf-8").splitlines(), 1):
            for m in CITE.finditer(l):
                c = cites[int(m.group(2), 16)]
                c["names"].add(m.group(1))
                c["at"].append(f"{rel}:{n}")
    tests = {p.relative_to(R).as_posix(): p.read_text(encoding="utf-8") for p in sorted((R / "godot" / "tests").rglob("test_*.gd"))}
    by_name = {}
    for rva in sorted(cites):
        c = cites[rva]
        names = sorted(c["names"])
        rid = f"RULE_FN_{rva:x}"
        hx = f"0x{rva:x}"
        tids = sorted(t for t, s in tests.items() if re.search(rf"\b{hx}\b", s) or any(nm in s for nm in names))
        area = c["at"][0].split("/")[2] if c["at"][0].startswith("godot/game/") else "records"
        b.rules[rid] = {"rule_id": rid, "rule_type": "ported_function", "name": names[0], "description": " = ".join(names),
                        "params": "{}", "field_refs": "", "source": f"{names[0]} rva={hx}", "implemented_by": ";".join(c["at"][:6]),
                        "area": area, "test_id": ";".join(tids[:6]), "status": "tested" if tids else "ported"}
        for nm in names:
            by_name.setdefault(nm, rid)
    rsrc = load("rules")
    for rid, rtype, desc, fns, pkey, frefs in CURATED:
        parts = []
        for fn in fns:
            if fn not in by_name:
                raise SystemExit(f"{rid}: {fn} is not cited by the port (godot/game, components/ue/records)")
            parts.append(by_name[fn])
        params = {"composes": parts}
        if pkey:
            params[pkey] = rsrc[pkey]
        tids = sorted({t for p in parts for t in b.rules[p]["test_id"].split(";") if t})
        b.rules[rid] = {"rule_id": rid, "rule_type": rtype, "name": rid[5:].lower(), "description": desc,
                        "params": json.dumps(params, sort_keys=True), "field_refs": ";".join(frefs),
                        "source": "; ".join(b.rules[p]["source"] for p in parts),
                        "implemented_by": ";".join(b.rules[p]["implemented_by"].split(";")[0] for p in parts),
                        "area": "spec", "test_id": ";".join(tids[:8]), "status": "tested" if tids else "ported"}
    perk_rules(b, cites)


def param_uses(names):
    """UE property name -> ["Function rva=0x.."] of every native decomp function that reads / writes it (->Name / .Name)"""
    if not names:
        return {}
    hdr = re.compile(r"^// (\S.*?)\s+rva=0x([0-9a-f]+) size=")
    rx = re.compile(r"(?:->|\.)(" + "|".join(sorted(map(re.escape, names), key=len, reverse=True)) + r")\b")
    out = collections.defaultdict(list)
    for p in sorted((R / "extract" / "native" / "decomp").glob("*.cpp")):
        cur = None
        with open(p, encoding="utf-8", errors="replace") as f:
            for l in f:
                m = hdr.match(l)
                if m:
                    cur = f"{m.group(1)} rva=0x{m.group(2)}"
                    continue
                if cur:
                    for u in rx.finditer(l):
                        if cur not in out[u.group(1)]:
                            out[u.group(1)].append(cur)
    return out


def perk_rules(b, cites):
    """RULE_PERK_<Name>: one rule per EPerk value, built from the native code only (the port takes HasPerk false, so
    status not_ported): every function that tests the perk (HasPerk(.., value), exact call lines), every function that
    reads one of the perk's UPerkSystemComponent parameters, and those parameters as field refs. A Rust port implements
    a perk by this rule ID: the listed sites are where the exe branches on it."""
    sites = getattr(has_perk_checks, "sites", {})
    uses = param_uses([p for info in b.perk_info for p in info["params"]])
    for info in sorted(b.perk_info, key=lambda x: x["value"]):
        rid = sid("RULE_PERK", info["stem"])
        calls = sites.get(info["value"], [])
        readers = sorted({f for prm in info["params"] for f in uses.get(prm, [])})
        fns = sorted({f"{s['function']} rva={s['rva']}" for s in calls} | set(readers))
        ported = sorted({f"RULE_FN_{int(f.rsplit('0x', 1)[1], 16):x}" for f in fns
                         if int(f.rsplit("0x", 1)[1], 16) in cites})
        params = {"enum_value": info["value"], "enum_name": info["enum"], "has_perk_sites": calls,
                  "parameter_readers": {prm: uses.get(prm, []) for prm in info["params"]},
                  "composes": ported}
        frefs = ["FLD_PERK_ENUM_VALUE"] + [sid("FLD_PERK", ("effect_" + snake(prm)).upper()) for prm in info["params"]]
        b.rules[rid] = {"rule_id": rid, "rule_type": "perk_effect", "name": info["stem"].lower(),
                        "description": f"{info['enum']} ({info['value']}): {len(calls)} HasPerk call site(s), "
                                       f"{len(info['params'])} parameter(s) read by {len(readers)} function(s)" +
                                       ("" if fns else "; no native site found (Blueprint-only perk, or tested by a "
                                        "non-literal argument: UNCONFIRMED)"),
                        "params": json.dumps(params, sort_keys=True), "field_refs": ";".join(frefs),
                        "source": "; ".join(fns) or "Z_Construct_UEnum_Mordhau_EPerk::Enumerators (exe)",
                        "implemented_by": "", "area": "perk", "test_id": "", "status": "not_ported"}
    known = {info["value"] for info in b.perk_info}
    for v in sorted(set(sites) - known):		# perk bits the exe tests that have no EPerk enumerator
        calls = sites[v]
        fns = sorted({f"{s['function']} rva={s['rva']}" for s in calls})
        rid = f"RULE_PERK_BIT_{v}"
        b.rules[rid] = {"rule_id": rid, "rule_type": "perk_effect", "name": f"perk_bit_{v}",
                        "description": f"perk bit {v}: tested by the exe but no EPerk enumerator has this value "
                                       f"(the reflection table ends at {max(known)}): a retired / legacy perk bit",
                        "params": json.dumps({"enum_value": v, "enum_name": "", "has_perk_sites": calls,
                                              "parameter_readers": {}, "composes": []}, sort_keys=True),
                        "field_refs": "", "source": "; ".join(fns), "implemented_by": "", "area": "perk", "test_id": "",
                        "status": "not_ported"}


# ---- evidence ----------------------------------------------------------------------------------------------------
PAR_FIELD = {"contact_time": "FLD_ATK_WINDUP", "feint_cost": "FLD_ATK_FEINT_COST", "morph_cost": "FLD_ATK_MORPH_COST",
             "miss_cost": "FLD_ATK_MISS_STAMINA_COST", "feint_lockout": "FLD_ATK_FEINT_LOCK_OUT"}
PAR_RULE = {"windup_release": "RULE_ATTACK_TIMELINE", "windup_release_stamina": "RULE_STAMINA", "miss_total": "RULE_ATTACK_TIMELINE",
            "combo_total": "RULE_ATTACK_TIMELINE", "misscombo_total": "RULE_ATTACK_TIMELINE", "morph_total": "RULE_ATTACK_TIMELINE",
            "riposte_total": "RULE_ATTACK_TIMELINE", "feint_time": "RULE_ATTACK_WINDOWS", "morph_time": "RULE_ATTACK_WINDOWS",
            "morph_time_min": "RULE_ATTACK_WINDOWS", "flinch_total": "RULE_FLINCH", "blocked_p2": "RULE_BLOCKED",
            "blocked_total": "RULE_BLOCKED", "block_drain": "RULE_STAMINA", "riposte_delay": "RULE_PARRY",
            "parry_up": "RULE_PARRY", "parry_up_block": "RULE_PARRY", "parry_fail_recovery": "RULE_PARRY",
            "parry_miss_recovery": "RULE_PARRY", "parry_success_recovery": "RULE_PARRY"}
# parry metrics (compare.py probes, tests/parity/probes.gd) -> the parry motion field that sets the measured span
# (game/combat/parry_motion.gd: parry_up_time line 69, parry_recovery_time 70 / fail, miss_parry_recovery_time 170,
# parry_success_recovery_time 184, riposte_window_base 214)
PARRY_FIELD = {"parry_up": "FLD_MOT_PARRY_UP_TIME", "parry_fail_recovery": "FLD_MOT_PARRY_RECOVERY_TIME",
               "parry_miss_recovery": "FLD_MOT_MISS_PARRY_RECOVERY_TIME",
               "parry_success_recovery": "FLD_MOT_PARRY_SUCCESS_RECOVERY_TIME", "riposte_delay": "FLD_MOT_RIPOSTE_WINDOW_BASE"}
PAR_STATUS = {"PASS": "observed", "net-PASS": "observed", "FAIL": "contradicted", "net-FAIL": "contradicted"}
# character / mode metrics (scripts/parity/char_mode_metrics.py): metric -> (entity type, field); the entity is the
# row's mode id (weapon column) or the character
PAR_ENTITY_FIELD = {"respawn_delay": ("mode", "FLD_MODE_SCORING_PLAYER_RESPAWN_TIME"),
                    "supersprint_speed": ("movement", "FLD_MOV_SUPERSPRINT_MODIFIER"),
                    "sprint_ratio_spec": ("movement", "FLD_MOV_SPRINT_MODIFIER"),
                    "backpedal_ratio_spec": ("movement", "FLD_MOV_BACKPEDAL_MODIFIER"),
                    "crouch_ratio_spec": ("movement", "FLD_MOV_MAX_WALK_SPEED_CROUCHED"),
                    "jump_vz_spec": ("movement", "FLD_MOV_JUMP_Z_VELOCITY"),
                    "chase_speed": ("movement", "FLD_MOV_CHASING_MODIFIER"),
                    "team_count": ("mode", "FLD_MODE_STATE_TEAM_COUNT"),
                    "is_team_mode": ("mode", "FLD_MODE_STATE_B_IS_TEAM_MODE"),
                    "move_sprint_ratio": ("movement", "FLD_MOV_SPRINT_MODIFIER"),
                    "move_diag_sprint_ratio": ("movement", "FLD_MOV_SPRINT_MODIFIER"),
                    "move_backpedal_ratio": ("movement", "FLD_MOV_BACKPEDAL_MODIFIER"),
                    "move_crouch_ratio": ("movement", "FLD_MOV_MAX_WALK_SPEED_CROUCHED"),
                    "jump_gravity": ("movement", "FLD_MOV_GRAVITY_SCALE"),
                    "jump_takeoff_vz": ("movement", "FLD_MOV_JUMP_Z_VELOCITY"),
                    "jump_apex": ("movement", "FLD_MOV_JUMP_Z_VELOCITY"),
                    "jump_air_time": ("movement", "FLD_MOV_JUMP_Z_VELOCITY"),
                    "skm_round_start": ("mode", "FLD_MODE_ROUND_START_DURATION"),
                    "skm_round_end": ("mode", "FLD_MODE_ROUND_END_DURATION"),
                    "skm_round_play_max": ("mode", "FLD_MODE_ROUND_DURATION"),
                    "skm_late_spawn": ("mode", "FLD_MODE_LATE_ROUND_SPAWN_DURATION"),
                    "stamina_regen_step": ("character", "FLD_CHR_STAMINA_REGEN_PER_TICK"),
                    "stamina_regen_interval": ("character", "FLD_CHR_STAMINA_REGEN_TICK_RATE"),
                    "stamina_regen_delay": ("character", "FLD_CHR_STAMINA_REGEN_DELAY")}


def evidence(b):
    tests = {p.relative_to(R).as_posix(): p.read_text(encoding="utf-8") for p in sorted((R / "godot" / "tests").rglob("test_*.gd"))}
    # one ledger row per field: the value is what the cited reader gives (a hypothesis about runtime behaviour); the
    # tests that read the field through the engine path are the ones that would verify or falsify it
    for fid, f in sorted(b.fields.items()):
        base = f["name"].split("_", 1)[1] if f["name"].startswith(("equip_", "rules_", "table_")) else f["name"]
        tids = sorted(t for t, s in tests.items() if len(base) > 3 and re.search(rf"\.{re.escape(base)}\b", s))
        n = sum(1 for v in b.values[f["entity_type"]].values() if fid in v)
        eid = "EVD_" + fid
        b.evidence[eid] = {"evidence_id": eid, "target_id": fid, "status": "hypothesis",
                           "observation": f"{n} {f['entity_type']} values of {f['name']} as the port's reader gives them",
                           "source": f["source"], "version": VERSION,
                           "procedure": "godot/tools/gen_spec_src.gd (components/ue readers) -> scripts/sheets_populate.py",
                           "expected": "", "observed": "", "test_id": ";".join(tids[:6])}
    for rid, r in sorted(b.rules.items()):
        eid = "EVD_" + rid
        b.evidence[eid] = {"evidence_id": eid, "target_id": rid, "status": "hypothesis",	# a port test is tier c, not an observation of the game
                           "observation": ("ported rule exercised by a headless test" if r["test_id"] else
                                           "ported from the cited function; no test cites it"),
                           "source": r["source"], "version": VERSION,
                           "procedure": "godot --headless tests/run.gd --only=<test_id>" if r["test_id"] else "",
                           "expected": "", "observed": "", "test_id": r["test_id"]}
    p = R / "state" / "parity" / "compare.json"
    if not p.exists():
        return
    sys.path.insert(0, str(R / "scripts"))
    import sheets_contradictions as sc		# the same probable-cause rules as state/spec_contradictions.md
    rows = sc.load_rows()					# compare.json + char_mode.json (character / mode metrics)
    cctx = sc.context(rows)
    for row in rows:
        kind = row["kind"].split(":")[0]
        w = row["weapon"]
        eid = sid("EVD", row["server"], w, row["kind"], row["metric"])
        if eid in b.evidence:
            eid = sid(eid, len(b.evidence))
        metric = row["metric"]
        if metric in PAR_ENTITY_FIELD:
            etype, fld = PAR_ENTITY_FIELD[metric]
            ent = sid("ENT", TYPES[etype][0], {"character": "BP_MordhauCharacter", "movement": "MOVEMENT"}.get(etype, w))
            target = f"{ent}:{fld}" if ent in b.entities else fld
        elif metric == "bot_footwork":
            target = "FLD_BOT_WILL_FOOTWORK"
        elif metric in PARRY_FIELD and not any(x in w for x in "+|"):
            # the parrier weapon's parry motion (FLD_WPN_PARRY_MOTION, else native UParryMotion) holds the field the
            # metric measures (game/combat/parry_motion.gd reads it: parry_up_time, parry_recovery_time, ...)
            went = sid("ENT_WPN", w)
            mot = (b.values["weapon"].get(went, {}).get("FLD_WPN_PARRY_MOTION") if went in b.entities else None) \
                or "ENT_MOT_NATIVE_UParryMotion"
            target = f"{mot}:{PARRY_FIELD[metric]}" if went in b.entities else PARRY_FIELD[metric]
        elif metric in PARRY_FIELD:
            target = PARRY_FIELD[metric]		# a left-hand item may override the parry motion: field level
        elif metric == "move_strafe_ratio":
            target = "ENT_MOV_MOVEMENT"		# StrafeModifier is an exe literal (UMordhauMovementComponent +0xcd4), no spec field
        elif metric in PAR_FIELD and kind in ("strike", "stab"):
            ent = sid('ENT_ATK', w, kind.upper())
            # a modded server's weapon (no package in this build) evidences the field, not one entity's value
            target = f"{ent}:{PAR_FIELD[metric]}" if ent in b.entities else PAR_FIELD[metric]
        else:
            target = PAR_RULE.get(metric, "")
        if not target:
            raise SystemExit(f"parity metric {metric} has no spec target")
        obs = row.get("obs", {})
        b.evidence[eid] = {"evidence_id": eid, "target_id": target, "status": PAR_STATUS.get(row["status"], "unknown"),
                           "observation": f"{row.get('what', metric)} [{row['status']}] weapon {w}" +
                                          (f" vs {pathlib.PurePosixPath(row['other']).name}" if row.get("other") else "") +
                                          (" | probable cause: %s (owner %s): %s" % sc.cause(row, cctx)
                                           if row["status"] in sc.FAILING else ""),
                           "source": "user demos (state/parity/%s, private)" % row.get("_file", "compare.json"),
                           "version": VERSION,
                           "procedure": {"char_mode.json": "scripts/parity/char_mode_metrics.py: decoded demos vs the spec value",
                                         "rust_demo_parity.json": "core/crates/mh-character/tests/demo_parity.rs (exe mode) vs "
                                                                  "scripts/parity/movement_metrics.py; parsed by rust_demo_parity.py",
                                         "movement_spec.json": "scripts/parity/movement_spec_metrics.py: decoded movement / "
                                                               "player-state stream vs the spec value"}.get(
                               row.get("_file"), "scripts/parity/extract_events.py -> compare.py; mode or core mean of n samples"),
                           "expected": "" if row.get("rewrite") is None else row["rewrite"],
                           "observed": obs.get("mode", obs.get("mean", obs.get("p02", obs.get(
                               "max" if metric == "skm_round_play_max" else "median", "")))),
                           "test_id": {"char_mode.json": "", "rust_demo_parity.json": "", "movement_spec.json": ""}.get(row.get("_file"),
                                                                                                   "godot/tests/test_demo_parity.gd")}


# ---- workbook ----------------------------------------------------------------------------------------------------
def report(b):
    print("coverage per entity type (entities / fields / values):")
    for et in TYPES:
        n = len(b.values[et]); nf = sum(1 for f in b.fields.values() if f["entity_type"] == et)
        nv = sum(len(v) for v in b.values[et].values())
        print(f"  {et:10s} {n:6d} {nf:5d} {nv:8d}")
    print("  (perk rows hold each perk's native HasPerk call sites + effect parameters; the port takes HasPerk false)")
    st = collections.Counter(e["status"] for e in b.evidence.values())
    print(f"rules {len(b.rules)} (curated {len(CURATED)}), evidence {len(b.evidence)}: " + ", ".join(f"{k} {v}" for k, v in sorted(st.items())))


SYSTEMS = [("combat", ["weapon", "attack", "motion"]), ("character / movement", ["character", "movement", "stat", "camera", "physics", "curve", "horse", "projectile", "ladder", "equipment_movement"]),
           ("perks", ["perk"]), ("modes / maps", ["mode", "mode_class", "squad", "horde", "map", "nav"]), ("AI", ["bot", "bt"]),
           ("loadout / cosmetics", ["wearable", "equipment", "skin", "color", "cosmetic"]),
           ("audio", ["sound_cue", "sound_att", "sound_conc", "sound_event"]), ("animation", ["anim"]),
           ("settings", ["setting"]), ("constants", ["constant"])]
RULE_AREAS = {"combat": ["combat", "spec", "actor"], "character / movement": ["character"], "perks": ["perk"],
              "modes / maps": ["mode", "mode_hrd", "mode_br"], "AI": ["ai"], "loadout / cosmetics": ["ui"],
              "animation": ["anim"], "settings": [], "audio": [], "constants": ["records", "net"]}
DOC = R / "docs" / "SPEC_SHEETS.md"


def coverage(b):
    """per gameplay system: fields, fields with OBSERVED evidence (demo parity or a test-exercised rule naming the
    field), fields linked to a test, contradicted evidence, rules and their tested share -> markdown table"""
    obs, contra = set(), collections.defaultdict(set)
    for e in b.evidence.values():
        t = e["target_id"]
        fids = [t.split(":", 1)[1]] if ":" in t else [t] if t.startswith("FLD_") else \
            [f for f in b.rules.get(t, {}).get("field_refs", "").split(";") if f]
        for f in fids:
            if e["status"] == "observed":
                obs.add(f)
            if e["status"] == "contradicted":
                contra[f].add(e["evidence_id"])
    tested = {e["target_id"] for e in b.evidence.values() if e["target_id"].startswith("FLD_") and e["test_id"]}
    rows = ["| system | entity types | entities | fields | observed | % observed | test-linked | contradicted rows | "
            "rules (tested) |", "|---|---|---:|---:|---:|---:|---:|---:|---|"]
    tot = [0, 0, 0, 0]
    for name, types in SYSTEMS:
        fids = [f for f, d in b.fields.items() if d["entity_type"] in types]
        nent = sum(len(b.values[t]) for t in types)
        if not fids and not nent:
            continue
        no = sum(1 for f in fids if f in obs)
        nt = sum(1 for f in fids if f in tested)
        nc = len(set().union(*[contra[f] for f in fids])) if fids else 0
        rl = [r for r in b.rules.values() if r["area"] in RULE_AREAS.get(name, [])]
        rt = sum(1 for r in rl if r["status"] == "tested")
        tot = [tot[0] + len(fids), tot[1] + no, tot[2] + nt, tot[3] + nent]
        rows.append(f"| {name} | {', '.join(t for t in types if b.values[t])} | {nent} | {len(fids)} | {no} | "
                    f"{100 * no / max(1, len(fids)):.1f} | {nt} | {nc} | {len(rl)} ({rt}) |")
    rows.append(f"| **all** | | {tot[3]} | {tot[0]} | {tot[1]} | {100 * tot[1] / max(1, tot[0]):.1f} | {tot[2]} | "
                f"{len(set().union(*contra.values())) if contra else 0} | {len(b.rules)} ({sum(1 for r in b.rules.values() if r['status'] == 'tested')}) |")
    return "\n".join(rows)


def write_coverage(b):
    start, end = "<!-- coverage:start (generated by scripts/sheets_populate.py) -->", "<!-- coverage:end -->"
    import sheets_tiers
    me = sys.modules[__name__]
    block = (f"{start}\n## Coverage: evidence tiers per system\n\n"
             "Each field and rule carries the tiers of proof it has (`tiers` column; rules in `data_gen/spec/rules.json`, "
             "fields in `fields.json`; how each is decided: `scripts/sheets_tiers.py`). A row counts in every tier it has.\n\n"
             "- **a shipped-data**: read verbatim from the game files by a verified reader (pak reader == CUE4Parse JSON "
             "on 874,770 values, native ctor / .rdata hash-checked, combat values == spec by ID). Exact data; behaviour "
             "not checked.\n"
             "- **b exe-code**: the native function is byte-matched in `src/` (tracker `byte_match` MATCH) or an exe-mode "
             "Rust test cites it.\n"
             "- **c port-tested**: a GDScript or Rust test exercises it (rva / name cited by a test, or ported in a Rust "
             "crate with golden-trace tests).\n"
             "- **d demo-observed**: real-match parity PASS (`state/parity`: compare, char_mode, rust_demo_parity). Only "
             "this tier is Evidence status `observed`.\n\n"
             f"{sheets_tiers.table(me, b)}\n\n"
             "### To do for the demo work: highest-value fields / rules with only tier a (or nothing)\n\n"
             "Ranked by how often the port reads the field (godot/game + core/crates src) or how many places implement "
             "the rule.\n\n"
             f"{sheets_tiers.todo(me, b)}\n{end}")
    s = DOC.read_text(encoding="utf-8")
    if start in s:
        s = s[:s.index(start)] + block + s[s.index(end) + len(end):]
    else:
        h = s.index("\n## ")
        s = s[:h] + "\n\n" + block + "\n" + s[h:]
    DOC.write_text(s, encoding="utf-8", newline="\n")


def main():
    check = "--check" in sys.argv
    b = populate()
    hdr, data = sheets(b)
    old = read_book(OUT)
    rep = diff(old, hdr, data)
    report(b)
    print(coverage(b))
    if getattr(b, "horde_spec_check", None):
        print("docs/HORDE_SPEC.md citations: %d resolve, %d do not" % b.horde_spec_check)
    if not check:
        write_coverage(b)
    if not rep:
        print(f"{OUT.relative_to(R)}: up to date ({sum(len(v) for v in data.values())} rows)")
        return 0
    print(f"{OUT.relative_to(R)}: {'STALE' if check else 'changed'}:")
    print("\n".join(rep))
    if check:
        return 1
    write_book(OUT, hdr, data)
    print(f"wrote {OUT.relative_to(R)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())

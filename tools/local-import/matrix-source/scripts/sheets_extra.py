# sheets_extra.py - the "every feature" extension of the spec workbook (sheets r5), called by sheets_populate.populate():
#   Horde + Battle Royale  merged class defaults (gen_spec_src modes_extra), Horde squad data assets, one rule per
#                          Blueprint bytecode function (scripts/kismet/pp.py dumps in state/sheets/kismet), and a check
#                          of every citation in docs/HORDE_SPEC.md against that bytecode / the decomp / the JSON
#   cosmetics             weapon skins, colour tables, emblems / badges / voices / eyebrows (gen_spec_src cosmetics)
#   audio                 every SoundCue / SoundAttenuation / SoundConcurrency (gen_spec_src audio, UeSound reader =
#                          mh-assets sound_cue.rs) and the gameplay properties that name a cue (raw package JSON),
#                          with the native functions that read each such property
# Nothing here is typed in: every value comes from those dumps / files, every rule cites its package:function.
import collections, json, os, pathlib, re, subprocess, sys

R = pathlib.Path(__file__).resolve().parents[1]
KIS = R / "state" / "sheets" / "kismet"
GMODES = "Mordhau/Content/Mordhau/Blueprints/GameModes/"
KIS_PKGS = {"HRD": ["Horde/BP_HordeGameMode", "Horde/BP_HordeGameState"],
            "BR": ["BattleRoyale/BP_BattleRoyaleGameMode", "BattleRoyale/BP_BattleRoyaleGameState"]}
HORDE_SPEC = R / "docs" / "HORDE_SPEC.md"
FN_HDR = re.compile(r"^// (\S.*?)\s*$")		# function names may hold spaces ("Find Squad")
STMT = re.compile(r"^\s*(\d+)\s\s(.*)$")
CALL = re.compile(r"([A-Za-z_][A-Za-z0-9_ ]*?)\(")


def kismet_dump(pkg):
    """state/sheets/kismet/<name>.txt (scripts/kismet/pp.py over extract/raw), made when missing"""
    KIS.mkdir(parents=True, exist_ok=True)
    out = KIS / (pkg.rsplit("/", 1)[-1] + ".txt")
    if not out.exists():
        subprocess.run([sys.executable, str(R / "scripts" / "kismet" / "pp.py"), str(R / "extract" / "raw"),
                        GMODES + pkg, str(out)], check=True)
    funcs, cur = {}, None
    for l in out.read_text(encoding="utf-8").splitlines():
        m = FN_HDR.match(l)
        if m:
            cur = m.group(1)
            funcs[cur] = []
            continue
        s = STMT.match(l)
        if s and cur:
            funcs[cur].append((int(s.group(1)), s.group(2)))
    return out, funcs


def run(sp, b):
    motions_raw(sp, b)
    horde_br(sp, b)
    cosmetics(sp, b)
    audio(sp, b)


# ---- Horde / Battle Royale ------------------------------------------------------------------------------------------
def motions_raw(sp, b):
    """UAttackMotion / UParryMotion properties the Rust combat port reads but the typed MotionDefs records do not carry
    (rust-combat request, sheets r7: CheckAttackParry rva=0x1616cd0, CheckClash rva=0x16179a0, CheckParry
    rva=0x164ed40), merged native ctor + CDO chain (gen_spec_src motions_raw), set on the motion entities; a motion
    class without the property has no value"""
    d = sp.load_opt("motions_raw")
    if not d:
        return
    for key, props in sorted(d["motions"].items()):
        eid = sp.sid("ENT_MOT", "NATIVE_" + key[7:]) if key.startswith("native:") else sp.sid("ENT_MOT", key.rsplit("/", 1)[-1])
        if eid not in b.entities:
            continue
        for k, v in sorted(props.items()):
            st = "bool" if isinstance(v, bool) else "int" if k == "ActiveParryStaminaCost" else "float"
            b.set("motion", eid, sp.snake(k), v, st)


def horde_br(sp, b):
    ex = sp.load_opt("modes_extra")
    if ex:
        for p, d in sorted(ex["classes"].items()):
            mode = "HRD" if "/Horde/" in p else "BR"
            i = b.entity("mode_class", p.rsplit("/", 1)[-1], p.rsplit("/", 1)[-1],
                         b.source("pkg", p, "extract/json/" + p + ".json (+ native ctor, CombatData.class_defaults)"),
                         record="CombatData.class_defaults")
            b.set("mode_class", i, "mode", mode, "string")
            b.set("mode_class", i, "native", d.get("__native", ""), "string")
            ue_fields(sp, b, "mode_class", i, d)
        for p, d in sorted(ex["squads"].items()):
            i = b.entity("squad", p.rsplit("/", 1)[-1], p.rsplit("/", 1)[-1], b.source("pkg", p, "extract/json/" + p + ".json"),
                         record="USquadInfo data asset")
            ue_fields(sp, b, "squad", i, d)
    cited = horde_spec_cites()
    for mode, pkgs in KIS_PKGS.items():
        for pkg in pkgs:
            if not (R / "extract" / "raw" / (GMODES + pkg + ".uasset")).exists():
                continue
            path, funcs = kismet_dump(pkg)
            cls = pkg.rsplit("/", 1)[-1]
            for fn, stmts in sorted(funcs.items()):
                rid = sp.sid("RULE", mode, cls.replace("BP_", ""), fn)[:120]
                calls = collections.Counter()
                for _, s in stmts:
                    for c in CALL.findall(s):
                        c = c.strip().split(".")[-1]
                        if c and not c.startswith(("if ", "cast")) and c not in ("Cast",):
                            calls[c] += 1
                cites = cited.get((cls, fn), [])
                params = {"package": GMODES + pkg, "function": fn, "statements": len(stmts),
                          "first_index": stmts[0][0] if stmts else None, "last_index": stmts[-1][0] if stmts else None,
                          "calls": [c for c, _ in calls.most_common(40)],
                          "horde_spec_cites": cites}
                b.rules[rid] = {"rule_id": rid, "rule_type": "blueprint_function", "name": f"{cls}.{fn}",
                                "description": f"{mode} Blueprint function {cls}::{fn}: {len(stmts)} bytecode statements"
                                               + (f"; docs/HORDE_SPEC.md cites it {len(cites)}x" if cites else ""),
                                "params": json.dumps(params, sort_keys=True), "field_refs": "",
                                "source": f"{GMODES}{pkg}:{fn} (bytecode, {path.relative_to(R).as_posix()})",
                                "implemented_by": "", "area": "mode_" + mode.lower(), "test_id": "", "status": "not_ported"}
    verify_horde_spec(sp, b)
    mode_rule_ids(b)


MODE_RULE_IDS = "core/crates/mh-mode/src/rule_ids.rs"
RULE_TUPLE = re.compile(r'\(\s*"(RULE_[^"]+)",\s*"([^"]*)",\s*"([^"]*)",\s*"([^"]*)",\s*"((?:[^"\\]|\\.)*)",?\s*\)')


def mode_rule_ids(b):
    """implemented_by / test_id / status of the RULE_HRD_* / RULE_BR_* rows from mh-mode's own table (rust-mode-ai r4:
    `RULES: &[(rule id, status, implemented_by "src/x.rs::fn;..", test, note)]`, checked against the matrix by
    mh-mode tests/rule_ids.rs). Paths become repo-relative; an id the table has but the book does not is an error."""
    p = R / MODE_RULE_IDS
    if not p.exists():
        return
    crate = MODE_RULE_IDS.rsplit("/src/", 1)[0]
    for m in RULE_TUPLE.finditer(p.read_text(encoding="utf-8")):
        rid, status, impl, test, note = m.groups()
        note = note.replace('\\"', '"')	# the Rust literal's escaped quotes
        if rid not in b.rules:
            raise SystemExit(f"{MODE_RULE_IDS}: {rid} is not a spec rule")
        r = b.rules[rid]
        r["status"] = status
        r["implemented_by"] = ";".join(f"{crate}/{x}" for x in impl.split(";") if x)
        r["test_id"] = f"{crate}/{test}" if test else ""
        if note:
            r["description"] += f"; port note ({MODE_RULE_IDS}): {note}"


def ue_fields(sp, b, etype, i, d):
    for k, v in sorted(d.items()):
        if k.startswith("__"):
            continue
        if isinstance(v, dict) and "ObjectPath" in v:
            v = sp.strip_types(v.get("ObjectPath", ""))
        st = sp.infer(v)
        if isinstance(v, (dict, list)) and st != "float_list" and st != "string_list":
            st = "json"
        fid = sp.sid("FLD", sp.TYPES[etype][0], sp.snake(k).upper())
        have = b.fields.get(fid, {}).get("type")
        if have and have != st and {have, st} != {"int", "float"}:
            st = "json"			# raw UE values differ in shape between classes (native enum byte vs Blueprint enum name)
        b.set(etype, i, sp.snake(k), v, st)


def horde_spec_cites():
    """(class, function) -> [statement index] cited in docs/HORDE_SPEC.md (bare @N inherits the paragraph's last
    named function)"""
    out = collections.defaultdict(list)
    if not HORDE_SPEC.exists():
        return out
    for para in HORDE_SPEC.read_text(encoding="utf-8").split("\n\n"):
        last = None
        for m in re.finditer(r"`([A-Za-z_][\w ]*?)?@(\d+)", para):
            fn = m.group(1) or last
            if m.group(1):
                last = m.group(1)
            if fn:
                cls = "BP_HordeGameState" if "GameState" in fn else "BP_HordeGameMode"
                out[(cls, fn)].append((int(m.group(2)), bool(m.group(1))))
    return out


def verify_horde_spec(sp, b):
    """every docs/HORDE_SPEC.md citation checked against the bytecode dumps, extract/decomp and extract/json:
    one Evidence row per cited (function, index) / file line: observed = resolves, contradicted = does not"""
    if not HORDE_SPEC.exists():
        return
    funcs = {}
    for pkg in KIS_PKGS["HRD"]:
        if (R / "extract" / "raw" / (GMODES + pkg + ".uasset")).exists():
            _, f = kismet_dump(pkg)
            cls = pkg.rsplit("/", 1)[-1]
            for fn, st in f.items():
                funcs[(cls, fn)] = {i for i, _ in st}
    files = {"GM.cpp": R / "extract" / "decomp" / (GMODES + "Horde/BP_HordeGameMode.BP_HordeGameMode_C.cpp"),
             "GM.json": R / "extract" / "json" / (GMODES + "Horde/BP_HordeGameMode.json")}
    lines = {k: (sum(1 for _ in open(p, encoding="utf-8", errors="replace")) if p.exists() else 0) for k, p in files.items()}
    ok = bad = 0
    rid_spec = "RULE_HRD_HORDE_SPEC"
    for (cls, fn), idxs in sorted(horde_spec_cites().items()):
        have = funcs.get((cls, fn))
        if have is None:			# a bare index whose paragraph names no function: try the ubergraph of either class
            for c2 in ("BP_HordeGameMode", "BP_HordeGameState"):
                if (c2, fn) in funcs:
                    have, cls = funcs[(c2, fn)], c2
        rid = sp.sid("RULE_HRD", cls.replace("BP_", ""), fn)[:120]
        uber = funcs.get((cls, "ExecuteUbergraph_" + cls), set())
        for ix, explicit in sorted(set(idxs)):
            good = have is not None and ix in have
            # a bare `@N` inherits the paragraph's last named function; events (ReceiveTick, OnKilled) only jump into
            # the ubergraph, so a bare index that is an ubergraph statement is that body
            via_uber = not good and not explicit and ix in uber
            good = good or via_uber
            ok += good
            bad += not good
            eid = sp.sid("EVD_HORDE_SPEC", cls, fn, ix)
            b.evidence[eid] = {"evidence_id": eid, "target_id": rid if rid in b.rules else rid_spec,
                               "status": "hypothesis" if good else "contradicted",	# a valid citation is not a game observation
                               "observation": f"docs/HORDE_SPEC.md cites {fn}@{ix}" + (" (bare index)" if not explicit else "") +
                                              ": " + ("a statement index of that function's bytecode" if good and not via_uber
                                                      else "a statement index of the ubergraph (the event's body)" if via_uber
                                                      else "NOT a statement index of that function (or the function is "
                                                      f"not in {cls}'s bytecode): the citation is wrong"),
                               "source": "docs/HORDE_SPEC.md (Grok-written) vs scripts/kismet/pp.py dump",
                               "version": sp.VERSION, "procedure": "scripts/sheets_extra.py verify_horde_spec",
                               "expected": "", "observed": "", "test_id": ""}
    for k, mx in lines.items():
        for m in re.finditer(re.escape(k) + r":(\d+)", HORDE_SPEC.read_text(encoding="utf-8")):
            n = int(m.group(1))
            good = 0 < n <= mx
            ok += good
            bad += not good
            eid = sp.sid("EVD_HORDE_SPEC", k.replace(".", "_"), n)
            b.evidence[eid] = {"evidence_id": eid, "target_id": rid_spec, "status": "hypothesis" if good else "contradicted",	# a valid citation is not a game observation
                               "observation": f"docs/HORDE_SPEC.md cites {k}:{n}: " + ("line exists" if good else
                                              f"the file has {mx} lines: the citation is wrong"),
                               "source": files[k].relative_to(R).as_posix(), "version": sp.VERSION,
                               "procedure": "scripts/sheets_extra.py verify_horde_spec", "expected": "", "observed": "",
                               "test_id": ""}
    b.rules[rid_spec] = {"rule_id": rid_spec, "rule_type": "spec_document", "name": "horde_spec",
                         "description": f"docs/HORDE_SPEC.md (Grok-written Horde spec): {ok} citations resolve, {bad} do "
                                        "not (Evidence EVD_HORDE_SPEC_*)",
                         "params": json.dumps({"resolved": ok, "unresolved": bad}), "field_refs": "",
                         "source": "docs/HORDE_SPEC.md", "implemented_by": "", "area": "mode_hrd", "test_id": "",
                         "status": "not_ported"}
    b.horde_spec_check = (ok, bad)


# ---- cosmetics ------------------------------------------------------------------------------------------------------
def cosmetics(sp, b):
    c = sp.load_opt("cosmetics")
    if not c:
        return
    for wid, skins in sorted(c["skins"].items()):
        went = sp.sid("ENT_WPN", wid)
        for n, s in enumerate(skins):
            i = b.entity("skin", f"{wid}_{n:02d}", s.get("name", "") or f"{wid} skin {n}", b.entities[went]["source_id"]
                         if went in b.entities else b.source("record", "godot/components/ue/records/ue_weapon.gd"),
                         parent=went if went in b.entities else "", record="UeWeapon skins (FEquipmentSkinEntry)")
            b.set("skin", i, "weapon", went if went in b.entities else None, "ref", "weapon")
            b.set("skin", i, "index", n, "int")
            b.set("skin", i, "name", s.get("name", ""), "string")
            b.set("skin", i, "icon", s.get("icon", ""), "string")
            b.set("skin", i, "part_types", s.get("part_types", []), "json")
            b.set("skin", i, "n_parts", sum(len(t.get("parts", [])) for t in s.get("part_types", [])), "int")
    ssrc = b.source("pkg", "Mordhau/Content/Mordhau/Blueprints/BP_MordhauSingleton",
                    "extract/json/Mordhau/Content/Mordhau/Blueprints/BP_MordhauSingleton.json")
    for ti, row in enumerate(c.get("colors", [])):
        for ei, e in enumerate(row):
            i = b.entity("color", f"T{ti}_{ei:03d}", e["class"].rsplit("/", 1)[-1], ssrc, record="UeWearable.color")
            b.set("color", i, "table", ti, "int")
            b.set("color", i, "entry", ei, "int")
            b.set("color", i, "class", e["class"], "string")
            b.set("color", i, "color", e["color"], "color")
    kinds = {}
    for k in ("Emblems", "Badges", "MaleVoices", "FemaleVoices", "Eyebrows"):
        for n, ref in enumerate(c["singleton"].get(k) or []):
            p = sp.strip_types(ref.get("ObjectPath", "")) if isinstance(ref, dict) else str(ref)
            p = p.rsplit(".", 1)[0] if p.rsplit(".", 1)[-1].isdigit() else p
            kinds.setdefault(p, (k, n))
    for p, (k, n) in sorted(kinds.items()):
        d = c["classes"].get(p)
        if d is None:
            continue
        i = b.entity("cosmetic", p.rsplit("/", 1)[-1], p.rsplit("/", 1)[-1], b.source("pkg", p, "extract/json/" + p + ".json"),
                     record="BP_MordhauSingleton." + k)
        b.set("cosmetic", i, "kind", k, "string")
        b.set("cosmetic", i, "index", n, "int")
        ue_fields(sp, b, "cosmetic", i, {kk: v for kk, v in d.items()})
    for k in ("SkinColorTable", "EyeColorTable", "HairColorTable", "EmblemColorTable", "MetalTintsColorTable"):
        v = c["singleton"].get(k)
        if v is not None:
            i = b.entity("cosmetic", "TABLE_" + k, k, ssrc, record="BP_MordhauSingleton." + k)
            b.set("cosmetic", i, "kind", "color_table", "string")
            b.set("cosmetic", i, "table", v, "json")


# ---- audio ----------------------------------------------------------------------------------------------------------
SOUND_CLASSES = ("SoundCue'", "SoundWave'", "SoundBase'")


def audio(sp, b):
    a = sp.load_opt("audio")
    if not a:
        return
    att_ids = {}
    for p, d in sorted(a["attenuation"].items()):
        i = b.entity("sound_att", p.rsplit("/", 1)[-1], p.rsplit("/", 1)[-1], b.source("pkg", p, "extract/json/" + p + ".json"),
                     record="UeSound.attenuation (mh-assets sound_cue::attenuation)")
        att_ids[p] = i
        for k, v in sorted((d or {}).items()):
            b.set("sound_att", i, sp.snake(k) if k[:1].isupper() else k, v, "json" if isinstance(v, (dict, list)) else sp.infer(v))
    for p, d in sorted(a["concurrency"].items()):
        i = b.entity("sound_conc", p.rsplit("/", 1)[-1], p.rsplit("/", 1)[-1], b.source("pkg", p, "extract/json/" + p + ".json"),
                     record="SoundConcurrency export properties")
        for k, v in sorted((d or {}).items()):
            b.set("sound_conc", i, sp.snake(k), v, "json" if isinstance(v, (dict, list)) else sp.infer(v))
    cue_ids = {}
    for p, d in sorted(a["cues"].items()):
        if not d:
            continue
        key = p.rsplit("/", 1)[-1]
        if sp.sid("ENT_CUE", key) in b.entities:
            key = sp.sid(key, len(cue_ids))
        i = b.entity("sound_cue", key, p.rsplit("/", 1)[-1], b.source("pkg", p, "extract/json/" + p + ".json"),
                     record="UeSound.cue (mh-assets sound_cue::cue)")
        cue_ids[p] = i
        b.set("sound_cue", i, "package", p, "string")
        b.set("sound_cue", i, "volume", float(d.get("volume", 0.0)), "float")
        b.set("sound_cue", i, "pitch", float(d.get("pitch", 0.0)), "float")
        b.set("sound_cue", i, "exact", bool(d.get("exact", False)), "bool")
        b.set("sound_cue", i, "unknown_nodes", list(d.get("unknown", [])), "string_list")
        b.set("sound_cue", i, "waves", list(d.get("waves", [])), "string_list")
        b.set("sound_cue", i, "n_waves", len(d.get("waves", [])), "int")
        b.set("sound_cue", i, "attenuation", d.get("attenuation"), "json")
        b.set("sound_cue", i, "root", d.get("root"), "json")
    # which gameplay property names which cue: raw package JSON of Blueprints + Animations (CDOs, component templates,
    # AnimNotify_PlaySound objects); the property is the event, the native functions reading it are its call sites
    refs = []
    for top in ("Blueprints", "Animations"):
        root = R / "extract" / "json" / "Mordhau" / "Content" / "Mordhau" / top
        for fp in sorted(root.rglob("*.json")):
            t = fp.read_text(encoding="utf-8", errors="replace")
            if "SoundCue'" not in t and "SoundWave'" not in t:
                continue
            owner = fp.relative_to(R / "extract" / "json").with_suffix("").as_posix()
            for e in json.loads(t):
                for path, cue in walk(e.get("Properties", {}), ""):
                    refs.append((owner, e.get("Name", ""), e.get("Type", ""), path, cue))
    props = sorted({re.sub(r"\[\d+\]", "", path).split(".")[0] for _, _, _, path, _ in refs})
    readers = sp_param_uses(sp, props)
    seen = set()
    for owner, exp, typ, path, cue in refs:
        key = sp.sid(owner.rsplit("/", 1)[-1], exp, path)[:110]
        n = 0
        while key in seen:
            n += 1
            key = sp.sid(key[:100], n)
        seen.add(key)
        prop = re.sub(r"\[\d+\]", "", path).split(".")[0]
        i = b.entity("sound_event", key, f"{owner.rsplit('/', 1)[-1]}.{exp}.{path}", b.source("pkg", owner, "extract/json/" + owner + ".json"),
                     record="package JSON sound reference")
        b.set("sound_event", i, "owner", owner, "string")
        b.set("sound_event", i, "export", exp, "string")
        b.set("sound_event", i, "export_type", typ, "string")
        b.set("sound_event", i, "property", path, "string")
        b.set("sound_event", i, "sound", cue, "string")
        b.set("sound_event", i, "cue", cue_ids.get(cue), "ref", "sound_cue")
        b.set("sound_event", i, "native_readers", readers.get(prop, []), "string_list")
    b.sound_event_count = len(refs)


def walk(v, path):
    if isinstance(v, dict):
        on = str(v.get("ObjectName", ""))
        if on.startswith(SOUND_CLASSES) and "ObjectPath" in v:
            p = str(v["ObjectPath"])
            yield path, (p.rsplit(".", 1)[0] if p.rsplit(".", 1)[-1].isdigit() else p)
            return
        for k, x in v.items():
            yield from walk(x, f"{path}.{k}" if path else k)
    elif isinstance(v, list):
        for n, x in enumerate(v):
            yield from walk(x, f"{path}[{n}]")


def sp_param_uses(sp, names):
    return sp.param_uses([n for n in names if re.match(r"^[A-Za-z_]\w*$", n)])

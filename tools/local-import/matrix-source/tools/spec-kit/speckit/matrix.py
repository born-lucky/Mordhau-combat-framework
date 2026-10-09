# speckit.matrix - the Spreadsheet Method's "generate, do not hand-copy" step (game-agnostic): read the workbook, fail
# on duplicate IDs, broken references, invalid literals and schema gaps, write the generated matrix JSON (one file per
# entity type + fields / rules / evidence / sources / index), and --check that a saved matrix is up to date.
# Python stdlib + openpyxl. A game sets its format name and repo root with configure(); test_id paths in the workbook
# are checked relative to that root.
import hashlib, json, math, pathlib, re, struct, sys

import openpyxl

ROOT = pathlib.Path.cwd()   # test_id paths resolve against it (configure)
FORMAT = "spec/1"


def configure(root=None, fmt=None):
    global ROOT, FORMAT
    if root is not None:
        ROOT = pathlib.Path(root)
    if fmt is not None:
        FORMAT = fmt
PREFIX = {"Sources": "SRC_", "Entities": "ENT_", "Fields": "FLD_", "Rules": "RULE_", "Evidence": "EVD_"}
COLS = {
    "Sources": ["source_id", "kind", "path", "cite", "version"],
    "Entities": ["entity_id", "entity_type", "name", "parent_id", "source_id", "record"],
    "Fields": ["field_id", "entity_type", "name", "type", "ref_type", "unit", "unit_source", "source", "description", "tiers",
               "tier_note"],
    "Rules": ["rule_id", "rule_type", "name", "description", "params", "field_refs", "source", "implemented_by", "area",
              "test_id", "status", "tiers", "tier_note"],
    "Evidence": ["evidence_id", "target_id", "status", "observation", "source", "version", "procedure", "expected",
                 "observed", "test_id"],
    "Blobs": ["blob_id", "part", "text"],
}
TYPES = {"bool", "int", "float", "double", "string", "ref", "vec2", "vec3", "color", "float_list", "int_list",
         "string_list", "json"}
EVIDENCE_STATUS = {"observed", "hypothesis", "contradicted", "unknown", "illustrative"}
TIERS = {"a", "b", "c", "d"}	# evidence tiers (tools/spec-kit/README.md): shipped-data, exe-code, port-tested, demo-observed
# not_ported: specified from the exe, no port implements it yet; host: the decision logic is ported, the engine-side
# effect (spawning, spatial queries, visuals) is handed to the host as an event
RULE_STATUS = {"ported", "tested", "not_ported", "host"}
ID = re.compile(r"^[A-Za-z0-9_]+$")
F32_OVERFLOW = 3.4028235677973366e38	# FLT_MAX + half an ulp: below it a decimal rounds to a finite f32


class Errors(list):
    def add(self, where, msg):
        self.append(f"{where}: {msg}")


def text(v):
    return "" if v is None else str(v)


def read(path):
    wb = openpyxl.load_workbook(path, read_only=True, data_only=True)
    out = {}
    for ws in wb.worksheets:
        rows = ws.iter_rows(values_only=True)
        h = list(next(rows, []))
        while h and h[-1] is None: h.pop()
        body = []
        for n, r in enumerate(rows, 2):
            r = (list(r) + [None] * len(h))[:len(h)]
            if any(x is not None for x in r):
                body.append((n, r))
        out[ws.title] = (h, body)
    wb.close()
    return out


def f32(x):
    return struct.unpack("<f", struct.pack("<f", x))[0]


def num(v, where, err, integral=False, single=True):
    if isinstance(v, bool) or not isinstance(v, (int, float)):
        err.add(where, f"expected a number, got {v!r}"); return None
    if isinstance(v, float) and not math.isfinite(v):
        err.add(where, f"non-finite {v}"); return None
    if integral:
        if isinstance(v, float) and not v.is_integer():
            err.add(where, f"expected an integer, got {v!r}"); return None
        return int(v)
    if single:
        if abs(v) >= F32_OVERFLOW:
            err.add(where, f"{v!r} is outside f32"); return None
        # a spec float is an IEEE binary32 value (tools/spec-kit/README.md "Numbers"): emit its shortest round-trip decimal
        t = f32(float(v))
        for d in range(1, 10):
            c = float(f"{t:.{d}g}")
            if abs(c) < F32_OVERFLOW and f32(c) == t:
                return c
        return t
    return float(v)


def literal(v, ftype, where, err, blobs):
    """workbook cell -> typed JSON value (None = field not set for this entity)"""
    if v is None:
        return None
    if isinstance(v, str) and v.startswith("@blob:"):
        bid = v[6:]
        if bid not in blobs:
            err.add(where, f"missing blob {bid}"); return None
        blobs[bid]["used"] = True
        v = blobs[bid]["text"]
    if ftype == "bool":
        if not isinstance(v, bool): err.add(where, f"expected TRUE/FALSE, got {v!r}"); return None
        return v
    if ftype == "int": return num(v, where, err, integral=True)
    if ftype == "float": return num(v, where, err)
    if ftype == "double":
        try:
            x = float(v) if isinstance(v, str) else v
        except ValueError:
            err.add(where, f"expected a decimal, got {v!r}"); return None
        return num(x, where, err, single=False)
    if ftype in ("string", "ref"):
        if isinstance(v, bool) or not isinstance(v, (str, int, float)):
            err.add(where, f"expected text, got {v!r}"); return None
        s = str(v)
        if s.startswith('"'):
            try:
                s = json.loads(s)
            except ValueError:
                err.add(where, f"bad quoted string {v!r}"); return None
            if not isinstance(s, str): err.add(where, f"quoted literal is not a string: {v!r}"); return None
        if ftype == "ref":
            return s or None
        return s
    if not isinstance(v, str):
        err.add(where, f"expected a JSON literal, got {v!r}"); return None
    try:
        x = json.loads(v)
    except ValueError as e:
        err.add(where, f"bad JSON ({e}): {v[:80]!r}"); return None
    if ftype == "json":
        return x
    if not isinstance(x, list):
        err.add(where, f"expected a JSON list, got {v[:80]!r}"); return None
    n = {"vec2": 2, "vec3": 3, "color": 4}.get(ftype)
    if n is not None and len(x) != n:
        err.add(where, f"{ftype} needs {n} numbers, got {len(x)}"); return None
    if ftype == "string_list":
        if not all(isinstance(s, str) for s in x): err.add(where, "string_list holds a non-string"); return None
        return x
    out = [num(a, where, err, integral=ftype == "int_list") for a in x]
    return None if any(a is None for a in out) else out


OBJECT_MARKER = re.compile(r"^<[A-Za-z_][A-Za-z0-9_]*#-?\d+>$")	# Godot's str(Object): "<RefCounted#-922..>"


def has_object_marker(v):
    """a dumped value that is an engine object's default string form instead of its fields (a serializer depth cut)"""
    if isinstance(v, str):
        return bool(OBJECT_MARKER.match(v))
    if isinstance(v, list):
        return any(has_object_marker(x) for x in v)
    if isinstance(v, dict):
        return any(has_object_marker(x) for x in v.values())
    return False


def rows_of(book, name, err):
    if name not in book:
        err.add(name, "sheet missing"); return []
    h, body = book[name]
    if [text(x) for x in h] != COLS[name]:
        err.add(name, f"columns {h} != {COLS[name]}"); return []
    return [(n, dict(zip(COLS[name], r))) for n, r in body]


def split(v):
    return [x for x in text(v).split(";") if x]


def build(book):
    err = Errors()
    ids = {}

    def claim(i, where, prefix):
        if not isinstance(i, str) or not ID.match(i) or not i.startswith(prefix):
            err.add(where, f"bad id {i!r} (want {prefix}[A-Za-z0-9_]+)"); return False
        if i in ids:
            err.add(where, f"duplicate id {i} (first at {ids[i]})"); return False
        ids[i] = where
        return True

    sources, entities, fields, rules, evidence, blobs = {}, {}, {}, {}, {}, {}
    for n, r in rows_of(book, "Blobs", err):
        bid = text(r["blob_id"])
        if not ID.match(bid) or not bid.startswith("BLOB_"): err.add(f"Blobs!{n}", f"bad blob id {bid!r}"); continue
        blobs.setdefault(bid, {"parts": {}, "used": False})["parts"][r["part"]] = text(r["text"])
    for bid, b in blobs.items():
        parts = b["parts"]
        if sorted(parts) != list(range(len(parts))): err.add("Blobs", f"{bid}: parts {sorted(parts)} not 0..{len(parts) - 1}")
        b["text"] = "".join(parts[k] for k in sorted(parts) if isinstance(k, int))
    for n, r in rows_of(book, "Sources", err):
        if claim(r["source_id"], f"Sources!{n}", "SRC_"):
            if not text(r["kind"]) or not text(r["path"]): err.add(f"Sources!{n}", "kind and path are required")
            sources[r["source_id"]] = {k: text(r[k]) for k in ("kind", "path", "cite", "version")}
    sheets = {name[2:] for name in book if name.startswith("T_")}
    for n, r in rows_of(book, "Fields", err):
        w = f"Fields!{n}"
        if not claim(r["field_id"], w, "FLD_"): continue
        t = text(r["type"])
        if t not in TYPES: err.add(w, f"unknown type {t!r}")
        if text(r["entity_type"]) not in sheets: err.add(w, f"entity type {r['entity_type']!r} has no T_ sheet")
        if (t == "ref") != bool(text(r["ref_type"])): err.add(w, "ref_type is required for ref fields only")
        if t == "ref" and text(r["ref_type"]) not in sheets: err.add(w, f"ref_type {r['ref_type']!r} has no T_ sheet")
        if not text(r["name"]): err.add(w, "name is required")
        if not set(split(r["tiers"])) <= TIERS: err.add(w, f"tiers {r['tiers']!r} not a ;-list of {sorted(TIERS)}")
        fields[r["field_id"]] = {k: text(r[k]) for k in COLS["Fields"][1:]}
        fields[r["field_id"]]["tiers"] = split(r["tiers"])
    for n, r in rows_of(book, "Entities", err):
        w = f"Entities!{n}"
        if not claim(r["entity_id"], w, "ENT_"): continue
        if text(r["entity_type"]) not in sheets: err.add(w, f"entity type {r['entity_type']!r} has no T_ sheet")
        if text(r["source_id"]) not in sources: err.add(w, f"unknown source {r['source_id']!r}")
        entities[r["entity_id"]] = {"type": text(r["entity_type"]), "name": text(r["name"]), "parent": text(r["parent_id"]) or None,
                                    "source": text(r["source_id"]), "record": text(r["record"]), "values": None}
    for i, e in entities.items():
        if e["parent"] and e["parent"] not in entities: err.add(f"Entities {i}", f"unknown parent {e['parent']}")
    # value sheets
    for et in sorted(sheets):
        h, body = book["T_" + et]
        w0 = "T_" + et
        if not h or h[0] != "entity_id": err.add(w0, "first column must be entity_id"); continue
        cols = [text(x) for x in h[1:]]
        want = sorted(f for f, d in fields.items() if d["entity_type"] == et)
        for c in cols:
            if c not in fields: err.add(w0, f"column {c} is not a field")
            elif fields[c]["entity_type"] != et: err.add(w0, f"column {c} belongs to {fields[c]['entity_type']}")
        for c in set(want) - set(cols): err.add(w0, f"field {c} has no column")
        if len(set(cols)) != len(cols): err.add(w0, "duplicate columns")
        for n, r in body:
            eid = r[0]
            w = f"{w0}!{n}"
            if eid not in entities: err.add(w, f"unknown entity {eid!r}"); continue
            e = entities[eid]
            if e["type"] != et: err.add(w, f"{eid} is a {e['type']}, not a {et}"); continue
            if e["values"] is not None: err.add(w, f"second row for {eid}"); continue
            vals = {}
            for c, v in zip(cols, r[1:]):
                if c not in fields: continue
                x = literal(v, fields[c]["type"], f"{w} {c}", err, blobs)
                if x is not None and fields[c]["type"] in ("json", "string", "string_list") and has_object_marker(x):
                    err.add(f"{w} {c}", "holds an unserialized engine object (\"<Class#id>\"): the dump cut a nested record")
                if x is not None:
                    if fields[c]["type"] == "ref" and (x not in entities or entities[x]["type"] != fields[c]["ref_type"]):
                        err.add(f"{w} {c}", f"ref {x} is not a {fields[c]['ref_type']} entity")
                    vals[c] = x
            e["values"] = vals
    for i, e in entities.items():
        if e["values"] is None: err.add(f"Entities {i}", f"no row in T_{e['type']}"); e["values"] = {}
    for bid, b in blobs.items():
        if not b["used"]: err.add("Blobs", f"{bid} is not referenced")
    # one ID code per entity type: every FLD_<CODE>_* and ENT_<CODE>_* of the type shares it (tools/spec-kit/README.md "IDs")
    codes = {}
    for fid, f in fields.items():
        c = fid.split("_")[1] if fid.count("_") >= 2 else ""
        if codes.setdefault(f["entity_type"], c) != c:
            err.add(f"Fields {fid}", f"ID code {c} != {codes[f['entity_type']]} of entity type {f['entity_type']}")
    for i, e in entities.items():
        c = i.split("_")[1] if i.count("_") >= 2 else ""
        if codes.get(e["type"], c) != c:
            err.add(f"Entities {i}", f"ID code {c} != {codes[e['type']]} of entity type {e['type']}")
    if len(set(codes.values())) != len(codes):
        err.add("Fields", f"two entity types share an ID code: {codes}")
    # rules + evidence
    tests_ok = lambda t: (ROOT / t).is_file()
    for n, r in rows_of(book, "Rules", err):
        w = f"Rules!{n}"
        if not claim(r["rule_id"], w, "RULE_"): continue
        try:
            params = json.loads(text(r["params"]) or "{}")
        except ValueError as e:
            err.add(w, f"params: bad JSON ({e})"); params = {}
        if not isinstance(params, dict): err.add(w, "params must be a JSON object"); params = {}
        if text(r["status"]) not in RULE_STATUS: err.add(w, f"status {r['status']!r} not in {sorted(RULE_STATUS)}")
        if not text(r["source"]): err.add(w, "source citation is required")
        for f in split(r["field_refs"]):
            if f not in fields: err.add(w, f"field_refs: unknown field {f}")
        for t in split(r["test_id"]):
            if not tests_ok(t): err.add(w, f"test {t} does not exist")
        rules[r["rule_id"]] = {"type": text(r["rule_type"]), "name": text(r["name"]), "description": text(r["description"]),
                               "params": params, "fields": split(r["field_refs"]), "source": text(r["source"]),
                               "implemented_by": split(r["implemented_by"]), "area": text(r["area"]),
                               "tests": split(r["test_id"]), "status": text(r["status"]),
                               "tiers": split(r["tiers"]), "tier_note": text(r["tier_note"])}
        if not set(split(r["tiers"])) <= TIERS: err.add(w, f"tiers {r['tiers']!r} not a ;-list of {sorted(TIERS)}")
    for i, r in rules.items():
        for c in r["params"].get("composes", []):
            if c not in rules: err.add(f"Rules {i}", f"composes unknown rule {c}")
    for n, r in rows_of(book, "Evidence", err):
        w = f"Evidence!{n}"
        if not claim(r["evidence_id"], w, "EVD_"): continue
        t = text(r["target_id"])
        if ":" in t:
            e, f = t.split(":", 1)
            if e not in entities or f not in fields or fields[f]["entity_type"] != entities[e]["type"]:
                err.add(w, f"target {t}: not an entity:field pair of one type")
        elif t not in entities and t not in fields and t not in rules:
            err.add(w, f"unknown target {t!r}")
        if text(r["status"]) not in EVIDENCE_STATUS: err.add(w, f"status {r['status']!r} not in {sorted(EVIDENCE_STATUS)}")
        for x in split(r["test_id"]):
            if not tests_ok(x): err.add(w, f"test {x} does not exist")
        evidence[r["evidence_id"]] = {k: (r[k] if k in ("expected", "observed") and isinstance(r[k], (int, float)) else text(r[k]))
                                      for k in COLS["Evidence"][1:] if k != "test_id"}
        evidence[r["evidence_id"]]["tests"] = split(r["test_id"])
    return err, sources, entities, fields, rules, evidence


def dumps(x):
    return json.dumps(x, sort_keys=True, indent=1, ensure_ascii=False, allow_nan=False) + "\n"


def outputs(book):
    err, sources, entities, fields, rules, evidence = build(book)
    if err:
        return err, None
    files = {}
    by_type = {}
    for i, e in entities.items():
        by_type.setdefault(e["type"], {})[i] = {k: e[k] for k in ("name", "parent", "source", "record", "values")}
    for et, ents in by_type.items():
        files[f"entities/{et}.json"] = dumps(ents)
    files["fields.json"] = dumps(fields)
    files["rules.json"] = dumps(rules)
    files["evidence.json"] = dumps(evidence)
    files["sources.json"] = dumps(sources)
    h = hashlib.sha1()
    for k in sorted(files):
        h.update(k.encode()); h.update(files[k].encode())
    status = {}
    for e in evidence.values(): status[e["status"]] = status.get(e["status"], 0) + 1
    files["index.json"] = dumps({
        "format": FORMAT, "content_sha1": h.hexdigest(),
        "entity_types": {et: {"file": f"entities/{et}.json", "entities": len(ents),
                              "code": next(i.split("_")[1] for i in ents),
                              "fields": sum(1 for f in fields.values() if f["entity_type"] == et),
                              "values": sum(len(e["values"]) for e in ents.values())} for et, ents in sorted(by_type.items())},
        "counts": {"sources": len(sources), "entities": len(entities), "fields": len(fields), "rules": len(rules),
                   "evidence": len(evidence)},
        "evidence_status": status})
    return err, files


def _find(book, ftype):
    """(sheet, column index) of the first T_ column whose field has this type, with a non-empty first row"""
    fields = {}
    for n, r in rows_of(book, "Fields", Errors()):
        fields[r["field_id"]] = text(r["type"])
    for name, (h, body) in book.items():
        if not name.startswith("T_") or not body:
            continue
        for i, c in enumerate(h[1:], 1):
            if fields.get(text(c)) == ftype and body[0][1][i] is not None:
                return name, i
    return None, None


def selftest(book):
    """tamper with the in-memory workbook: each defect class must be reported (a validator that passes anything fails).
    Works on any workbook: the columns to tamper with are found by type."""
    import copy
    cases = []

    def dup(b):		# a duplicate entity id
        h, body = b["Entities"]; body.append(body[0])

    def badref(b):		# an entity pointing at a parent that does not exist
        h, body = b["Entities"]; i = h.index("parent_id"); body[0][1][i] = "ENT_NO_SUCH_ENTITY"

    def badlit(b):		# a float cell holding text
        s, i = _find(b, "float"); b[s][1][0][1][i] = "fast"

    def badvec(b):		# a vec2 with three numbers
        s, i = _find(b, "vec2"); b[s][1][0][1][i] = "[1,2,3]"

    def badobj(b):		# a json cell where the dump wrote an engine object's string instead of its fields
        s, i = _find(b, "json"); b[s][1][0][1][i] = '[{"node": "<RefCounted#-12345>"}]'

    def badfld(b):		# evidence on a field that does not exist
        h, body = b["Evidence"]; i = h.index("target_id"); body[0][1][i] = "FLD_NO_SUCH_FIELD"

    def badtype(b):		# an unknown field type
        h, body = b["Fields"]; i = h.index("type"); body[0][1][i] = "quaternion"

    def badentref(b):	# a ref cell naming an entity of the wrong type
        s, i = _find(b, "ref")
        h, body = b[s]
        rt = next(text(r["ref_type"]) for n, r in rows_of(b, "Fields", Errors()) if text(r["field_id"]) == text(h[i]))
        other = next(r["entity_id"] for n, r in rows_of(b, "Entities", Errors()) if text(r["entity_type"]) != rt)
        body[0][1][i] = other

    probes = [(dup, "duplicate id", None), (badref, "unknown parent", None), (badlit, "expected a number", "float"),
              (badvec, "vec2 needs 2", "vec2"), (badobj, "unserialized engine object", "json"),
              (badfld, "unknown target", None), (badtype, "unknown type", None), (badentref, "is not a", "ref")]
    for fn, want, needs in probes:
        if needs and _find(book, needs)[0] is None:
            print(f"  skip {fn.__name__}: the workbook has no {needs} column")
            continue
        b = copy.deepcopy(book)
        fn(b)
        err, _ = outputs(b)
        ok = any(want in e for e in err)
        cases.append(ok)
        print(f"  {'caught' if ok else 'MISSED'} {fn.__name__}: {want}" + ("" if ok else f" (errors: {err[:2]})"))
    return bool(cases) and all(cases)


def main(argv, workbook=None, out=None):
    check = "--check" in argv
    wbp = pathlib.Path(argv[argv.index("--workbook") + 1]) if "--workbook" in argv else pathlib.Path(workbook or "spec.xlsx")
    out = pathlib.Path(argv[argv.index("--out") + 1]) if "--out" in argv else pathlib.Path(out or "data_gen/spec")
    if not wbp.exists():
        print(f"FAIL: no workbook {wbp} (python scripts/sheets_populate.py)"); return 1
    book = read(wbp)
    if "--selftest" in argv:
        ok = selftest(book)
        print(("PASS" if ok else "FAIL") + ": build_matrix selftest")
        return 0 if ok else 1
    err, files = outputs(book)
    if err:
        print(f"FAIL: {len(err)} workbook error(s):")
        for e in err[:50]: print("  " + e)
        return 1
    idx = json.loads(files["index.json"])
    have = {p.relative_to(out).as_posix() for p in out.rglob("*.json")} if out.exists() else set()
    stale = sorted(k for k in files if not (out / k).exists() or (out / k).read_text(encoding="utf-8") != files[k])
    extra = sorted(have - set(files))
    summary = (f"{idx['counts']['entities']} entities, {idx['counts']['fields']} fields, {idx['counts']['rules']} rules, "
               f"{idx['counts']['evidence']} evidence, content {idx['content_sha1'][:12]}")
    if check:
        if stale or extra:
            print(f"FAIL: {out} is stale (rebuild without --check): changed/missing {stale[:5]} extra {extra[:5]}")
            return 1
        print(f"PASS: spec matrix up to date: {summary}")
        return 0
    for k in stale:
        (out / k).parent.mkdir(parents=True, exist_ok=True)
        (out / k).write_text(files[k], encoding="utf-8", newline="\n")
    for k in extra:
        (out / k).unlink()
    print(f"wrote {len(stale)} file(s), removed {len(extra)} -> {out}: {summary}")
    return 0

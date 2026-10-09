# sheets_tiers.py - evidence TIERS per field and rule (sheets r6), so "how proven is this" is not one mixed number:
#   a  shipped-data   the value is read verbatim from the game files (paks / extract json / .rdata / bytecode) by a
#                     verified reader: pak reader == CUE4Parse json on 874,770 values (test_pak_equiv), native ctor
#                     replay / .rdata literals hash-checked (verify_constants, test_boundary), combat values == spec by
#                     ID (mh-spec equivalence). Exact data, behaviour not checked. Derived fields (counts, refs the spec
#                     script computes) are not tier a.
#   b  exe-code       the rule's native function is byte-matched in src/ (state/tracker/Functions.tsv byte_match =
#                     MATCH), or an exe-mode test of the Rust port cites the function (core/crates/mh-character/tests/
#                     exe_*.rs); a field gets b through a rule with tier b that lists it in field_refs
#   c  port-tested    a GDScript test (godot/tests) or a Rust test (core/crates/*/tests) cites the function's rva / name,
#                     or the field is read by such a test; or the rva is cited in the src of a Rust crate that has
#                     golden-trace tests (tests/golden*.rs: the whole tick trace is compared, so every ported function on
#                     the path is exercised) - that last kind is listed as "c:golden-crate" in the tier note
#   d  demo-observed  real-match parity (state/parity: compare.json, char_mode.json, rust_demo_parity.json) PASS
#                     ("observed" Evidence keeps meaning only this)
import bisect, collections, pathlib, re, sys

R = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(R / "tools" / "spec-kit"))
from speckit import tiers as kit_tiers  # noqa: E402  (the generic assigner; this file supplies the inputs)
IMAGE_BASE = 0x140000000
DERIVED = re.compile(r"^(n_|native_default_ue_fields$|checked_by$|native_readers$|used_by_modes$|mode_prefix$|scene$|"
                     r"motion_|alt_motion_|attack_|parry_motion$|alt_parry_motion$|weapon$|cue$|kick_weapon$)")
ADDR = re.compile(r"\b0x(14[0-9a-fA-F]{7}|1[0-9a-fA-F]{6})\b")


class Funcs:
    """state/tracker/Functions.tsv: rva ranges -> (byte_match, name)"""
    def __init__(self):
        self.starts, self.rows = [], []
        p = R / "state" / "tracker" / "Functions.tsv"
        if not p.exists():
            return
        with open(p, encoding="utf-8") as f:
            hdr = next(f).rstrip("\n").split("\t")
            ix = {k: i for i, k in enumerate(hdr)}
            seen = {}
            for l in f:
                c = l.rstrip("\n").split("\t")
                rva, size = int(c[ix["rva"]], 16), int(c[ix["size"]] or 0)
                bm = c[ix["byte_match"]]
                if rva in seen:
                    if bm == "MATCH":
                        self.rows[seen[rva]] = (rva, size, bm, c[ix["name"]])
                    continue
                seen[rva] = len(self.rows)
                self.rows.append((rva, size, bm, c[ix["name"]]))
        self.rows.sort()
        self.starts = [r[0] for r in self.rows]

    def at(self, a):
        """the function containing rva a (or starting at it)"""
        i = bisect.bisect_right(self.starts, a) - 1
        if i >= 0:
            rva, size, bm, nm = self.rows[i]
            if rva <= a < rva + max(size, 1):
                return rva, bm
        return None

    def matched(self):
        return {r[0] for r in self.rows if r[2] == "MATCH"}


def rust_cites(fn):
    """function rva -> set of tier notes, from Rust tests (c:rust-test, b:exe-test) and golden-crate src (c:golden-crate)"""
    out = collections.defaultdict(set)
    crates = R / "core" / "crates"
    for crate in sorted(crates.iterdir()) if crates.exists() else []:
        tests = crate / "tests"
        golden = tests.exists() and any(p.name.startswith("golden") for p in tests.glob("*.rs"))
        for p in sorted(crate.rglob("*.rs")):
            rel = p.relative_to(crate).as_posix()
            if rel.startswith("tests/"):
                tag = "b:exe-test" if crate.name == "mh-character" and p.name.startswith("exe_") else "c:rust-test"
            elif rel.startswith("src/") and golden:
                tag = "c:golden-crate"
            else:
                continue
            for m in ADDR.finditer(p.read_text(encoding="utf-8", errors="replace")):
                a = int(m.group(1), 16)
                a = a - IMAGE_BASE if a >= IMAGE_BASE else a
                hit = fn.at(a)
                if hit:
                    out[hit[0]].add(f"{tag} ({crate.name}/{rel})")
    return out


def rust_field_reads():
    """snake field names a Rust test reads (`.name` or "name")"""
    names = collections.Counter()
    for p in sorted((R / "core" / "crates").glob("*/tests/*.rs")):
        for m in re.finditer(r"[.\"]([a-z][a-z0-9_]{3,})\b", p.read_text(encoding="utf-8", errors="replace")):
            names[m.group(1)] += 1
    return names


def compute(sp, b):
    """this game's tier inputs, assigned by the spec-kit's generic assigner (tools/spec-kit/speckit/tiers.py)"""
    fn = Funcs()
    matched = fn.matched()
    rc = rust_cites(fn)
    rfr = rust_field_reads()
    kinds = collections.defaultdict(set)		# entity type -> source kinds of its entities
    for e in b.entities.values():
        kinds[e["entity_type"]].add(b.sources.get(e["source_id"], {}).get("kind", ""))

    def shipped(f):
        ks = kinds[f["entity_type"]] & {"pkg", "native", "rdata", "kismet", "ini", "record"}
        if not DERIVED.match(f["name"]) and ks:
            return f"read by {f['source'] or 'a reader'} ({'/'.join(sorted(ks))})"
        return None

    def rule_functions(r):
        m = re.match(r"RULE_FN_([0-9a-f]+)$", r["rule_id"])
        return [int(m.group(1), 16)] if m else [int(x, 16) for x in re.findall(r"rva=0x([0-9a-f]+)", r["source"])]

    function_tests = {a: [(n[0], n[2:]) for n in sorted(notes)] for a, notes in rc.items()}

    def field_tests(f):
        out = []
        ev = b.evidence.get("EVD_" + f["field_id"])
        if ev and ev["test_id"]:
            out.append("godot test " + ev["test_id"].split(";")[0])
        base = f["name"].split("_", 1)[1] if f["name"].startswith(("equip_", "rules_", "table_", "scoring_", "state_")) else f["name"]
        if rfr.get(base):
            out.append(f"rust tests read .{base} ({rfr[base]}x)")
        return out

    def observed(e):		# tier d = a passing real-game measurement (state/parity), nothing else
        return "parity" in (e.get("source") or "") + (e.get("procedure") or "")

    return kit_tiers.compute(b, shipped, rule_functions, matched, function_tests, field_tests, observed,
                             test_prefix="godot test ", measure_word="parity row(s)")


def table(sp, b):
    return kit_tiers.table(b, sp.SYSTEMS, sp.RULE_AREAS)


def todo(sp, b, n=20):
    """highest-value fields / rules with only tier a or nothing: value = how often the port reads the field
    (godot/game + core/crates src) / cites the rule's function (implementation sites)"""
    uses = sp.code_uses()
    rust = collections.Counter()
    for p in sorted((R / "core" / "crates").glob("*/src/**/*.rs")):
        for m in re.finditer(r"\.([a-z][a-z0-9_]{3,})\b", p.read_text(encoding="utf-8", errors="replace")):
            rust[m.group(1)] += 1
    cand = []
    for fid, f in b.fields.items():
        if f.get("tiers", "") not in ("", "a") or f["type"] in ("string", "string_list", "json", "ref") or                 f["name"] in ("id", "index", "table", "entry") or f["unit"] == "index":
            continue			# identifiers, not behaviour
        base = f["name"].split("_", 1)[1] if f["name"].startswith(("equip_", "rules_", "table_", "scoring_", "state_")) else f["name"]
        v = len(uses.get(base, [])) + rust.get(base, 0)
        if v:
            cand.append((v, fid, f"{f['entity_type']}.{f['name']}", f"read {len(uses.get(base, []))}x in godot/game, "
                         f"{rust.get(base, 0)}x in core/crates src; unit {f['unit'] or '?'}"))
    for rid, r in b.rules.items():
        if r.get("tiers", "") not in ("", "a") or r["rule_type"] not in ("ported_function", "lookup", "timeline", "window",
                                                                          "stamina_cost", "stat", "modifier", "restriction"):
            continue
        v = len([x for x in r["implemented_by"].split(";") if x])
        if v:
            cand.append((v, rid, r["name"], f"{v} implementation site(s), {r['source'][:80]}"))
    cand.sort(key=lambda x: (-x[0], x[1]))
    out = ["| # | id | what | why it matters |", "|---:|---|---|---|"]
    for k, (v, i, nm, why) in enumerate(cand[:n], 1):
        out.append(f"| {k} | `{i}` | {nm} | {why} |")
    return "\n".join(out)

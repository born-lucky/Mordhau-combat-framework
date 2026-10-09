# speckit.tiers - evidence tiers per field and rule (game-agnostic), so "how proven is this" is not one mixed number:
#   a  shipped-data   the value is read verbatim from the game's files by a reader whose output is equivalence-tested
#   b  exe-code       the rule's native function is byte-matched in the decomp's src/ (a byte-match tracker), or an
#                     exe-mode test of the port cites it
#   c  port-tested    a test of the port exercises the rule / reads the field
#   d  demo-observed  a real-game measurement (parity) passes; Evidence status "observed" means only this
#
# Inputs (all plain data, the game's adapters compute them):
#   shipped(field_row) -> note | None          is this field read verbatim by a verified reader (tier a)
#   rule_functions(rule_row) -> [int]          the native function addresses a rule ports (empty for none)
#   byte_matched: set[int]                     addresses byte-matched in src/ (the tracker)
#   function_tests: {int: [(tier, note)]}      per address, test citations: ("b", ...) exe-mode, ("c", ...) port tests
#   field_tests(field_row) -> [note]           tests that read the field (tier c)
#   observed(evidence_row) -> bool             a passing real-game measurement (tier d), e.g. parity source
# Rules composing other rules (params.composes) inherit their tiers; a field inherits b / c from rules listing it in
# field_refs, and d from an observed Evidence row that targets it (directly, as "ENT:FLD", or through a rule).
import collections, json


def compute(book, shipped, rule_functions, byte_matched, function_tests, field_tests, observed,
            test_prefix="test ", measure_word="measurement(s)"):
    """test_prefix / measure_word: the wording of the generated tier notes (a game may keep its own)"""
    tiers = {}
    for rid, r in book.rules.items():
        t = collections.defaultdict(list)
        for a in rule_functions(r):
            if a in byte_matched:
                t["b"].append(f"byte-matched 0x{a:x}")
            for tier, note in function_tests.get(a, ()):
                t[tier].append(note)
        if r.get("test_id"):
            t["c"].append(test_prefix + r["test_id"].split(";")[0])
        tiers[rid] = t
    for rid, r in book.rules.items():
        for c in json.loads(r.get("params") or "{}").get("composes", []):
            for k, v in tiers.get(c, {}).items():
                if v and k not in tiers[rid]:
                    tiers[rid][k].append(f"via {c}")
    obs = collections.defaultdict(list)
    for e in book.evidence.values():
        if e["status"] == "observed" and observed(e):
            obs[e["target_id"]].append(e["evidence_id"])
    for t, eids in obs.items():
        ids = [t] if ":" not in t else [t.split(":", 1)[1]]
        if t in book.rules:
            tiers[t]["d"].append(f"{len(eids)} {measure_word}")
            ids = [f for f in book.rules[t]["field_refs"].split(";") if f]
        for f in ids:
            if f in book.fields:
                tiers.setdefault(f, collections.defaultdict(list))["d"].append(f"{len(eids)} {measure_word} via {t}")
    for fid, f in book.fields.items():
        t = tiers.setdefault(fid, collections.defaultdict(list))
        note = shipped(f)
        if note:
            t["a"].append(note)
        for n in field_tests(f):
            t["c"].append(n)
    for rid, r in book.rules.items():
        for k in ("b", "c"):
            if tiers[rid].get(k):
                for f in [x for x in r["field_refs"].split(";") if x in book.fields]:
                    tiers[f][k].append(f"via {rid}")
    for i, t in tiers.items():
        row = book.fields.get(i) or book.rules.get(i)
        if row is not None:
            row["tiers"] = ";".join(k for k in "abcd" if t.get(k))
            row["tier_note"] = " | ".join(f"{k}: {t[k][0]}" + (f" (+{len(t[k]) - 1})" if len(t[k]) > 1 else "")
                                          for k in "abcd" if t.get(k))[:500]
    book.tiers = tiers
    return tiers


def table(book, systems, rule_areas):
    """markdown: per system (name, [entity types]) the fields / rules per tier; rule_areas: system -> [rule area]"""
    rows = ["| system | fields | a shipped-data | b exe-code | c port-tested | d demo-observed | none | rules | rules b | "
            "rules c | rules d |", "|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|"]
    tot = collections.Counter()
    pct = lambda n, d: f"{n} ({100 * n / max(1, d):.0f}%)"
    for name, types in systems:
        fids = [f for f, d in book.fields.items() if d["entity_type"] in types]
        rl = [r for r in book.rules.values() if r["area"] in rule_areas.get(name, [])]
        if not fids and not rl:
            continue
        c = collections.Counter()
        for f in fids:
            ts = book.fields[f].get("tiers", "")
            for k in "abcd":
                c[k] += k in ts
            c["none"] += not ts
        rc = collections.Counter(k for r in rl for k in "bcd" if k in r.get("tiers", ""))
        rows.append(f"| {name} | {len(fids)} | {pct(c['a'], len(fids))} | {pct(c['b'], len(fids))} | {pct(c['c'], len(fids))} | "
                    f"{pct(c['d'], len(fids))} | {c['none']} | {len(rl)} | {rc['b']} | {rc['c']} | {rc['d']} |")
        tot.update(c)
        tot["f"] += len(fids)
        tot["r"] += len(rl)
        tot.update({"r" + k: v for k, v in rc.items()})
    rows.append(f"| **all** | {tot['f']} | {tot['a']} | {tot['b']} | {tot['c']} | {tot['d']} | {tot['none']} | {tot['r']} | "
                f"{tot['rb']} | {tot['rc']} | {tot['rd']} |")
    return "\n".join(rows)

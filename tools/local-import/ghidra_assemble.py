# ghidra_assemble.py - turn DecompGame.java's decompile journal into the per-class .cpp tree.
# usage: python scripts/ghidra_assemble.py <journal> <out dir> [game_functions.tsv] [--allow-timeouts]
# The journal (written group by group so a killed run can resume, see DecompGame.java journalDone) holds, per address
# group:  "@@G\t<key>\t<status>\t<n>\n", n x ("@@E\t<file>\t<utf8 bytes>\n" + bytes), "@@D\t<key>\n".
#
# Layout of each <Class>.cpp, so a reader meets code first:
#   1. a two-line file header (function counts)
#   2. the decompiled bodies of the class's own functions, largest first (ties: by rva)
#   3. compiler-generated bodies (exception-unwind funclets, static initializers), largest first
#   4. one-line entries for names whose body is printed elsewhere because the linker folded identical code
#      (/OPT:ICF): "// <name>  rva=... size=...\n// ICF alias of <canonical> (<File>.cpp)"
# Every function keeps its "// <name>  rva=0x.. size=.." header line (gate.py and readability.py count them).
# The full ICF membership (address -> canonical name + every folded name and its file) goes to one sidecar,
# <out dir>/_icf_aliases.txt, instead of being repeated in every class file; the printed body gets one line pointing
# there. Output depends only on the journal content (order of entries is fully determined by size/rva/name).
# Fails (exit 1) on missing groups or on run-dependent timeout bodies. Streams: only an index is held in memory.
import sys, pathlib, collections, re

R = pathlib.Path(__file__).resolve().parent.parent
HEAD = re.compile(r"^// (.*?)  rva=(0x[0-9a-f]+) size=(\d+)")


locals_of = {}   # key -> lines of _local_names.tsv for that address group
vcalls_of = {}   # key -> lines of _vcalls.tsv (rewritten virtual calls and their evidence) for that address group


def index(jp):
    """key -> [entry], entry = dict(file, off, n, name, rva, size, alias, gen); plus key -> status"""
    idx, st_of, status = {}, {}, collections.Counter()
    with open(jp, "rb") as f:
        cur = None; ents = []; st = None
        while True:
            l = f.readline()
            if not l: break
            c = l.rstrip(b"\n").decode("utf-8").split("\t")
            if c[0] == "@@G": cur, st, ents, loc, vc = c[1], c[2], [], "", ""
            elif c[0] == "@@E":
                n = int(c[2]); off = f.tell(); b = f.read(n)
                t = b.decode("utf-8")
                if c[1] == "@locals": loc = t; continue   # applied PDB local names of this group (side table)
                if c[1] == "@vcalls": vc = t; continue    # rewritten virtual calls of this group (side table)
                h = HEAD.match(t)
                name = h.group(1)
                ents.append(dict(file=c[1], off=off, n=n, name=name, rva=int(h.group(2), 16), size=int(h.group(3)),
                                 alias=bool(re.search(r"(?m)^// body: see ", t)),
                                 gen="[compiler-generated" in name))
            elif c[0] == "@@D" and c[1] == cur:
                idx[cur] = ents; st_of[cur] = st; locals_of[cur] = loc; vcalls_of[cur] = vc; status[st] += 1; cur = None   # a later record for the same key wins
            else: break
    return idx, st_of, status


def strip_icf(t):
    """drop the multi-line 'ICF-folded' listing DecompGame writes under the header"""
    lines = t.split("\n")
    out = [lines[0]]; i = 1
    if i < len(lines) and lines[i].startswith("// ICF-folded:"):
        i += 1
        while i < len(lines) and lines[i].startswith("//   "): i += 1
    return "\n".join(out + lines[i:])


def main():
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    jp, out = pathlib.Path(args[0]), pathlib.Path(args[1])
    tsv = pathlib.Path(args[2]) if len(args) > 2 else R / "extract" / "native" / "game_functions.tsv"
    order, seen = [], set()
    with open(tsv, encoding="utf-8") as h:
        next(h)
        for l in h:
            k = l.rstrip("\n").split("\t")[2]
            if k not in seen: seen.add(k); order.append(k)
    idx, st_of, status = index(jp)
    missing = [k for k in order if k not in idx]
    timeouts = [k for k in order if st_of.get(k, "").startswith("timeout")]
    if out.exists() and any(out.glob("*.cpp")):
        sys.exit(f"{out} already has .cpp files; assemble into an empty dir")
    out.mkdir(parents=True, exist_ok=True)

    per_file = collections.defaultdict(list)
    canon = {}   # key -> (name, file) of the entry whose body is printed
    for k in order:
        ents = idx.get(k, [])
        bodies = [e for e in ents if not e["alias"]]
        if bodies: canon[k] = (bodies[0]["name"].split(" [compiler-generated")[0], bodies[0]["file"])
        for e in ents:
            e["key"] = k; e["group"] = len(ents)
            per_file[e["file"]].append(e)

    def rank(e):
        return (2 if e["alias"] else 1 if e["gen"] else 0, -e["size"] if not e["alias"] else 0, e["rva"], e["name"])

    nent = 0
    with open(jp, "rb") as f:
        for fn in sorted(per_file):
            es = sorted(per_file[fn], key=rank)
            nb = sum(1 for e in es if not e["alias"] and not e["gen"])
            ng = sum(1 for e in es if not e["alias"] and e["gen"])
            na = sum(1 for e in es if e["alias"])
            parts = [f"// {fn}.cpp - Mordhau-Win64-Shipping.exe decompiled by Ghidra 11.3.2 with PDB names/types (scripts/ghidra_native.bat).\n"
                     f"// {nb} function bodies (largest first), then {ng} compiler-generated bodies, then {na} ICF aliases (see _icf_aliases.txt).\n\n"]
            for e in es:
                f.seek(e["off"]); t = f.read(e["n"]).decode("utf-8")
                hdr = t.split("\n", 1)[0]
                if e["alias"]:
                    cn, cf = canon.get(e["key"], ("?", "?"))
                    parts.append(f"{hdr}\n// ICF alias of {cn} ({cf}.cpp): identical machine code, body printed there\n\n")
                else:
                    t = strip_icf(t)
                    if e["group"] > 1:
                        a, b = t.split("\n", 1)
                        t = f"{a}\n// ICF: {e['group'] - 1} other names share this address (linker /OPT:ICF), listed in _icf_aliases.txt\n{b}"
                    parts.append(t)
                nent += 1
            (out / (fn + ".cpp")).write_bytes("".join(parts).encode("utf-8"))

    with open(out / "_local_names.tsv", "w", encoding="utf-8", newline="\n") as o:
        o.write("# PDB local names applied to decompiler variables (DecompGame.java localNames), one per line:\n"
                "# rva  name-in-output  PDB local  member offset (-1 = whole local)  frame (Ghidra stack offsets of RSP,RBP after the prologue)  checks\n"
                "# checks: ';'-joined  R,<reg>,<byte offset>,<size>,<pc>,<pcNext or -1 for a use>  |  K,<ghidra stack offset>,<size>,<pc>,<pcNext|-1>\n"
                "# Verified against extract/native/locals.tsv by scripts/check_local_names.py.\n")
        for k in sorted(order, key=lambda k: int(k)):
            if locals_of.get(k): o.write(locals_of[k])
    with open(out / "_vcalls.tsv", "w", encoding="utf-8", newline="\n") as o:
        o.write("# Virtual calls printed by name (DecompGame.java vcalls/rewrite), one per line:\n"
                "# rva  call site  vftable slot  printed name  class the slot was read from  cast printed (- = none)  evidence\n"
                "# evidence: R,<reg>,<text offset> = that class is the type of the PDB local live in <reg> there (the register the\n"
                "# vtable pointer was loaded through); V = the type of the object value itself (call return, member, parameter).\n"
                "# Verified against extract/native/locals.tsv and types/vtbl_types.json by scripts/check_local_names.py.\n")
        for k in sorted(order, key=lambda k: int(k)):
            if vcalls_of.get(k): o.write(vcalls_of[k])
    with open(out / "_icf_aliases.txt", "w", encoding="utf-8", newline="\n") as o:
        o.write("# Linker identical-code folding (/OPT:ICF): functions with identical machine code share one address.\n"
                "# Per address: rva, size, the name its body is printed under (and file), then every other name folded into it.\n"
                "# The other names' own source bodies are not recoverable from the binary.\n\n")
        for k in sorted((k for k in order if len(idx.get(k, [])) > 1), key=lambda k: int(k)):
            ents = idx[k]; cn, cf = canon.get(k, ("?", "?"))
            o.write(f"0x{0x1000 + int(k):x} size={ents[0]['size']} {cn}  [{cf}.cpp]\n")
            for e in sorted(ents, key=lambda e: (e["file"], e["name"])):
                if e["alias"]: o.write(f"    {e['name']}  [{e['file']}.cpp]\n")

    print(f"groups {len(idx)}/{len(order)} ({dict(status)}), function entries {nent}, files {len(per_file)}, missing groups {len(missing)}")
    if missing: print("missing:", missing[:10]); sys.exit(1)
    # a group whose typed decompile (or vtable-override re-decompile) timed out in THIS run: its body depends on the
    # run, so the tree is not reproducible. Fail; DecompGame printed the `rva level` line to add to skiptyped.txt.
    if timeouts and "--allow-timeouts" not in sys.argv:
        print(f"FAIL: {len(timeouts)} run-dependent timeout bodies (see TIMEOUT lines in the Ghidra log; add them to scripts/ghidra/skiptyped.txt):",
              [hex(0x1000 + int(k)) + " " + st_of[k] for k in timeouts[:10]]); sys.exit(1)


if __name__ == "__main__":
    main()

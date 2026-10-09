"""Generic MSF7/CodeView type decoder adapted from our scripts/native_types.py.
No game-derived type records or constructor values are embedded. Inputs and outputs stay local.
"""
import array, json, mmap, os, re, struct, pathlib, hashlib

class Msf:
    def __init__(s, path):
        s.f = open(path, "rb")
        sb = s.f.read(56)
        assert sb[:32] == b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0", "not an MSF 7.00 file"   # [MSF] SuperBlock.FileMagic
        s.bs, _fbm, _nb, ndir, _unk, bmaddr = struct.unpack_from("<6I", sb, 32)                   # [MSF] BlockSize..BlockMapAddr
        nblk = (ndir + s.bs - 1) // s.bs
        s.f.seek(bmaddr * s.bs)
        dblocks = struct.unpack(f"<{nblk}I", s.f.read(4 * nblk))                                  # [MSF] block map = dir block list
        d = b"".join(s._blk(b) for b in dblocks)[:ndir]
        n = struct.unpack_from("<I", d)[0]                                                         # [MSF] NumStreams
        s.sizes = list(struct.unpack_from(f"<{n}I", d, 4))                                         # [MSF] StreamSizes[]
        p, s.blocks = 4 + 4 * n, []
        for z in s.sizes:
            k = 0 if z == 0xFFFFFFFF else (z + s.bs - 1) // s.bs
            s.blocks.append(struct.unpack_from(f"<{k}I", d, p)); p += 4 * k                        # [MSF] StreamBlocks[][]
    def _blk(s, b):
        s.f.seek(b * s.bs); return s.f.read(s.bs)
    def read(s, i):  # small streams only
        return b"".join(s._blk(b) for b in s.blocks[i])[: s.sizes[i]]
    def export(s, i, path):  # big streams: copy block by block to a contiguous file
        path = pathlib.Path(path)
        if path.exists() and path.stat().st_size == s.sizes[i]: return path
        path.parent.mkdir(parents=True, exist_ok=True)
        left = s.sizes[i]
        with open(str(path) + ".tmp", "wb") as w:
            for b in s.blocks[i]:
                c = s._blk(b)[: min(s.bs, left)]; w.write(c); left -= len(c)
        os.replace(str(path) + ".tmp", path)
        return path

def named_streams(msf):
    """[PDBI] PDB stream (index 1): Version, Signature, Age, Guid, then the named stream map:
    u32 StringBufferSize, char[] Strings, then a [HASH] table: Size, Capacity, Present bitvector, Deleted bitvector,
    then (u32 key = offset into Strings, u32 value = stream index) for each present bucket."""
    d = msf.read(1); p = 28
    sl = struct.unpack_from("<I", d, p)[0]; p += 4
    strs = d[p:p + sl]; p += sl
    size, cap = struct.unpack_from("<II", d, p); p += 8
    pw = struct.unpack_from("<I", d, p)[0]; p += 4
    present = struct.unpack_from(f"<{pw}I", d, p); p += 4 * pw
    dw = struct.unpack_from("<I", d, p)[0]; p += 4 + 4 * dw
    out = {}
    for b in range(cap):
        if present[b // 32] >> (b % 32) & 1:
            k, v = struct.unpack_from("<II", d, p); p += 8
            out[strs[k:strs.index(b"\0", k)].decode()] = v
    return out

# ---------------------------------------------------------------- type stream [TPI]
LF = dict(MODIFIER=0x1001, POINTER=0x1002, PROCEDURE=0x1008, MFUNCTION=0x1009, VTSHAPE=0x000A,
          ARGLIST=0x1201, FIELDLIST=0x1203, BITFIELD=0x1205, METHODLIST=0x1206,
          BCLASS=0x1400, VBCLASS=0x1401, IVBCLASS=0x1402, INDEX=0x1404, VFUNCTAB=0x1409,
          ENUMERATE=0x1502, ARRAY=0x1503, CLASS=0x1504, STRUCTURE=0x1505, UNION=0x1506, ENUM=0x1507,
          MEMBER=0x150D, STMEMBER=0x150E, METHOD=0x150F, NESTTYPE=0x1510, ONEMETHOD=0x1511, INTERFACE=0x1519,
          STRING_ID=0x1605, UDT_SRC_LINE=0x1606, UDT_MOD_SRC_LINE=0x1607)            # [CVI] LEAF_ENUM_e
UDT = (LF["CLASS"], LF["STRUCTURE"], LF["INTERFACE"])
FWDREF, UNIQUE = 0x80, 0x200                                                                   # [CVI] CV_prop_t

class Tpi:
    """[TPI] header: Version, HeaderSize, TypeIndexBegin, TypeIndexEnd, TypeRecordBytes, ... (56 bytes).
    Records follow at HeaderSize: u16 RecordLen (not counting itself), u16 RecordKind, payload."""
    def __init__(s, path, want_udts):
        s.fh = open(path, "rb"); s.mm = mmap.mmap(s.fh.fileno(), 0, access=mmap.ACCESS_READ)
        _ver, hs, s.begin, s.end, nbytes = struct.unpack_from("<5I", s.mm, 0)
        s.off = array.array("Q"); s.complete = {}; s.dups = {}
        p, e, ti, mm, cu = hs, hs + nbytes, s.begin, s.mm, s.complete
        while p < e:
            ln, k = struct.unpack_from("<HH", mm, p)
            s.off.append(p)
            if want_udts and (k in UDT or k == LF["UNION"] or k == LF["ENUM"]):
                prop = struct.unpack_from("<H", mm, p + 6)[0]
                if not prop & FWDREF:
                    nm = s.key(ti)
                    if nm not in cu: cu[nm] = ti
                    elif cu[nm] != ti: s.dups.setdefault(nm, [cu[nm]]).append(ti)
            p += 2 + ln; ti += 1
        assert ti == s.end, (hex(ti), hex(s.end))
    def rec(s, ti):
        p = s.off[ti - s.begin]; ln, k = struct.unpack_from("<HH", s.mm, p)
        return k, p + 4, p + 2 + ln
    def num(s, p):
        """numeric leaf [CVI]: u16 < LF_NUMERIC(0x8000) is the value, else a typed value follows."""
        v = struct.unpack_from("<H", s.mm, p)[0]
        if v < 0x8000: return v, p + 2
        f = {0x8000: "b", 0x8001: "h", 0x8002: "H", 0x8003: "i", 0x8004: "I", 0x8009: "q", 0x800A: "Q"}[v]
        return struct.unpack_from("<" + f, s.mm, p + 2)[0], p + 2 + struct.calcsize(f)
    def cstr(s, p):
        q = s.mm.find(b"\0", p); return s.mm[p:q].decode("utf-8", "replace"), q + 1
    def key(s, ti):
        """Forward refs are resolved like the linker/LLVM do: by unique (decorated) name when the record has one
        (CV_prop_t.hasuniquename), else by name."""
        k, p, _e = s.rec(ti)
        if k in UDT:
            prop = struct.unpack_from("<H", s.mm, p + 2)[0]; _, q = s.num(p + 16); nm, q = s.cstr(q)
        elif k == LF["UNION"]:
            prop = struct.unpack_from("<H", s.mm, p + 2)[0]; _, q = s.num(p + 8); nm, q = s.cstr(q)
        else:
            prop = struct.unpack_from("<H", s.mm, p + 2)[0]; nm, q = s.cstr(p + 12)
        return s.cstr(q)[0] if prop & UNIQUE else nm
    def udt(s, ti):
        """lfClass/lfStructure [CVI]: u16 count, u16 property, u32 field, u32 derived, u32 vshape, numeric size,
        name, [unique name if CV_prop_t.hasuniquename]. lfUnion: count, property, field, numeric size, name.
        lfEnum: count, property, u32 utype, u32 field, name.  Returns (kind, name, prop, field, size, extra)."""
        k, p, _e = s.rec(ti)
        if k in UDT:
            cnt, prop, fld, _der, vsh = struct.unpack_from("<HHIII", s.mm, p)
            size, q = s.num(p + 16); nm, _ = s.cstr(q)
            return k, nm, prop, fld, size, vsh
        if k == LF["UNION"]:
            cnt, prop, fld = struct.unpack_from("<HHI", s.mm, p)
            size, q = s.num(p + 8); nm, _ = s.cstr(q)
            return k, nm, prop, fld, size, 0
        if k == LF["ENUM"]:
            cnt, prop, ut, fld = struct.unpack_from("<HHII", s.mm, p)
            nm, _ = s.cstr(p + 12)
            return k, nm, prop, fld, None, ut
        return None

def pad(mm, p, e):
    while p < e and mm[p] >= 0xF0: p += mm[p] & 0x0F          # LF_PAD0..15 [CVI]: low nibble = bytes to skip
    return p

# ---------------------------------------------------------------- type model
SIMPLE = {0x00: ("<notype>", 0), 0x03: ("void", 0), 0x08: ("HRESULT", 4), 0x10: ("int8", 1), 0x20: ("uint8", 1),
          0x68: ("int8", 1), 0x69: ("uint8", 1), 0x70: ("char", 1), 0x71: ("wchar_t", 2), 0x7A: ("char16_t", 2),
          0x7B: ("char32_t", 4), 0x7C: ("char8_t", 1), 0x11: ("int16", 2), 0x21: ("uint16", 2), 0x72: ("int16", 2),
          0x73: ("uint16", 2), 0x12: ("long", 4), 0x22: ("unsigned long", 4), 0x74: ("int32", 4), 0x75: ("uint32", 4),
          0x13: ("int64", 8), 0x23: ("uint64", 8), 0x76: ("int64", 8), 0x77: ("uint64", 8), 0x14: ("int128", 16),
          0x24: ("uint128", 16), 0x40: ("float", 4), 0x41: ("double", 8), 0x42: ("long double", 10),
          0x30: ("bool", 1), 0x31: ("bool16", 2), 0x32: ("bool32", 4), 0x33: ("bool64", 8)}   # [CVT] simple type kinds

class Types:
    def __init__(s, tpi):
        s.t = tpi; s.sz = {}; s.layout_cache = {}; s.vt_cache = {}
    # ---- resolve a forward reference to the complete definition by name
    def full(s, ti):
        if ti < 0x1000: return ti
        k, p, _ = s.t.rec(ti)
        if k in UDT or k in (LF["UNION"], LF["ENUM"]):
            u = s.t.udt(ti)
            if u[2] & FWDREF: return s.t.complete.get(s.t.key(ti), ti)
        return ti
    def size(s, ti):
        if ti < 0x1000:
            return 8 if (ti >> 8) & 7 else SIMPLE.get(ti & 0xFF, ("?", 0))[1]   # [CVT] mode bits 8-10: pointer
        if ti in s.sz: return s.sz[ti]
        k, p, e = s.t.rec(ti); m = s.t.mm; r = 0
        if k == LF["POINTER"]: r = (struct.unpack_from("<I", m, p + 4)[0] >> 13) & 0x3F              # [CVI] lfPointerAttr.size
        elif k == LF["MODIFIER"] or k == LF["BITFIELD"]: r = s.size(struct.unpack_from("<I", m, p)[0])
        elif k == LF["ARRAY"]: r = s.t.num(p + 8)[0]                                                 # [CVI] lfArray: elemtype, idxtype, size
        elif k in UDT or k == LF["UNION"]:
            f = s.full(ti); r = s.t.udt(f)[4]
        elif k == LF["ENUM"]: r = s.size(s.t.udt(s.full(ti))[5])
        s.sz[ti] = r; return r
    # ---- C declarator: decl(ti, "Name") -> "float Name[4]" etc.
    def decl(s, ti, inner=""):
        sp = (lambda t: t + (" " + inner if inner else ""))
        if ti < 0x1000:
            b = SIMPLE.get(ti & 0xFF, (f"simple_{ti:#x}", 0))[0]
            if (ti >> 8) & 7: return sp(b + "*")
            return sp(b)
        k, p, e = s.t.rec(ti); m = s.t.mm
        if k in UDT or k == LF["UNION"] or k == LF["ENUM"]: return sp(s.t.udt(ti)[1])
        if k == LF["MODIFIER"]:
            base, mods = struct.unpack_from("<IH", m, p)                                              # [CVI] lfModifier: type, CV_modifier_t
            q = ("const " if mods & 1 else "") + ("volatile " if mods & 2 else "")
            if base < 0x1000 or s.t.rec(base)[0] != LF["POINTER"]: return q + s.decl(base, inner)
            return s.decl(base, inner)  # const on pointer handled loosely
        if k == LF["POINTER"]:
            ref, attr = struct.unpack_from("<II", m, p)                                               # [CVI] lfPointer: utype, attr
            mode = (attr >> 5) & 7; c = " const" if attr & 0x400 else ""
            tok = {0: "*", 1: "&", 4: "&&"}.get(mode)
            if mode in (2, 3):  # pointer to member: + containing class
                cls = struct.unpack_from("<I", m, p + 8)[0]
                tok = s.decl(cls) + "::*"
            rk = s.t.rec(ref)[0] if ref >= 0x1000 else None
            wrap = rk in (LF["PROCEDURE"], LF["MFUNCTION"], LF["ARRAY"])
            x = tok + c + (" " + inner if inner and c else inner)
            return s.decl(ref, "(" + x + ")" if wrap else x)
        if k == LF["ARRAY"]:
            el = struct.unpack_from("<I", m, p)[0]; tot = s.t.num(p + 8)[0]; es = s.size(el) or 1
            return s.decl(el, f"{inner}[{tot // es}]")
        if k == LF["BITFIELD"]: return s.decl(struct.unpack_from("<I", m, p)[0], inner)
        if k == LF["PROCEDURE"]:
            ret, cc, fa, n, al = struct.unpack_from("<IBBHI", m, p)                                   # [CVI] lfProc
            return s.decl(ret, f"{inner}({s.args(al)})")
        if k == LF["MFUNCTION"]:
            ret, cls, this, cc, fa, n, al = struct.unpack_from("<IIIBBHI", m, p)                      # [CVI] lfMFunc
            return s.decl(ret, f"{inner}({s.args(al)})")
        if k == LF["VTSHAPE"]: return sp("vftable")
        return sp(f"type_{ti:#x}")
    def arglist(s, al):
        m = s.t.mm; k, p, e = s.t.rec(al)
        n = struct.unpack_from("<I", m, p)[0]                                                         # [CVI] lfArgList: count, arg[]
        return list(struct.unpack_from(f"<{n}I", m, p + 4))
    def args(s, al):
        return ", ".join("..." if a == 0 else s.decl(a) for a in s.arglist(al))
    # ---- field list walk (follows LF_INDEX continuations)
    def fields(s, fl):
        m = s.t.mm
        while fl:
            k, p, e = s.t.rec(fl); fl = 0
            assert k == LF["FIELDLIST"], hex(k)
            while p < e:
                p = pad(m, p, e)
                if p >= e: break
                lk = struct.unpack_from("<H", m, p)[0]; p += 2
                if lk == LF["BCLASS"]:                                                                # attr, index, offset
                    at, ti = struct.unpack_from("<HI", m, p); off, p = s.t.num(p + 6)
                    yield ("base", at, ti, off)
                elif lk in (LF["VBCLASS"], LF["IVBCLASS"]):                                           # attr, btype, vbptr, vbpoff, vboff
                    at, ti, vbp = struct.unpack_from("<HII", m, p); a, p = s.t.num(p + 10); b, p = s.t.num(p)
                    yield ("vbase", at, ti, a, b)
                elif lk == LF["MEMBER"]:                                                              # attr, index, offset, name
                    at, ti = struct.unpack_from("<HI", m, p); off, p = s.t.num(p + 6); nm, p = s.t.cstr(p)
                    yield ("member", at, ti, off, nm)
                elif lk == LF["STMEMBER"]:                                                            # attr, index, name
                    at, ti = struct.unpack_from("<HI", m, p); nm, p = s.t.cstr(p + 6)
                    yield ("static", at, ti, nm)
                elif lk == LF["METHOD"]:                                                              # count, mList, name
                    cnt, ml = struct.unpack_from("<HI", m, p); nm, p = s.t.cstr(p + 6)
                    for at, ft, vo in s.methodlist(ml): yield ("method", at, ft, vo, nm)
                elif lk == LF["ONEMETHOD"]:                                                           # attr, index, [vbaseoff], name
                    at, ft = struct.unpack_from("<HI", m, p); p += 6; vo = None
                    if (at >> 2) & 7 in (4, 6): vo = struct.unpack_from("<I", m, p)[0]; p += 4
                    nm, p = s.t.cstr(p)
                    yield ("method", at, ft, vo, nm)
                elif lk == LF["NESTTYPE"]:                                                            # pad, index, name
                    ti = struct.unpack_from("<I", m, p + 2)[0]; nm, p = s.t.cstr(p + 6)
                    yield ("nested", 0, ti, nm)
                elif lk == LF["VFUNCTAB"]:                                                            # pad, type
                    ti = struct.unpack_from("<I", m, p + 2)[0]; p += 6
                    yield ("vfptr", 0, ti)
                elif lk == LF["ENUMERATE"]:                                                           # attr, value, name
                    v, p = s.t.num(p + 2); nm, p = s.t.cstr(p)
                    yield ("enumerator", 0, v, nm)
                elif lk == LF["INDEX"]:                                                               # pad, continuation
                    fl = struct.unpack_from("<I", m, p + 2)[0]; p += 6
                else:
                    raise ValueError(f"unknown field leaf {lk:#x} in fieldlist {fl:#x}")
    def methodlist(s, ml):
        m = s.t.mm; k, p, e = s.t.rec(ml)
        while p < e:                                                                                  # [CVI] mlMethod: attr, pad0, index, [vbaseoff]
            at, _, ft = struct.unpack_from("<HHI", m, p); p += 8; vo = None
            if (at >> 2) & 7 in (4, 6): vo = struct.unpack_from("<I", m, p)[0]; p += 4
            yield at, ft, vo
    def sig(s, ft):
        """(return, [args], is_static, funcattr, this_ti) of an LF_MFUNCTION / LF_PROCEDURE."""
        k, p, e = s.t.rec(ft); m = s.t.mm
        if k == LF["MFUNCTION"]:
            ret, cls, this, cc, fa, n, al = struct.unpack_from("<IIIBBHI", m, p)
            return ret, s.arglist(al), this == 0, fa, this
        ret, cc, fa, n, al = struct.unpack_from("<IBBHI", m, p)
        return ret, s.arglist(al), True, fa, 0
    # ---- one UDT's layout
    def layout(s, ti):
        ti = s.full(ti)
        if ti in s.layout_cache: return s.layout_cache[ti]
        kind, name, prop, fl, size, ext = s.t.udt(ti)
        L = dict(ti=ti, kind={0x1504: "class", 0x1505: "struct", 0x1519: "interface", 0x1506: "union", 0x1507: "enum"}[kind],
                 name=name, size=size, bases=[], vbases=[], members=[], statics=[], methods=[], vfptr=False,
                 enumerators=[], packed=bool(prop & 1))
        if kind == LF["ENUM"]:
            L["size"] = s.size(ext); L["utype"] = s.decl(ext)
        for f in (s.fields(fl) if fl else ()):
            if f[0] == "base": L["bases"].append(dict(ti=f[2], name=s.decl(f[2]), off=f[3]))
            elif f[0] == "vbase": L["vbases"].append(dict(ti=f[2], name=s.decl(f[2])))
            elif f[0] == "member": L["members"].append(dict(ti=f[2], off=f[3], name=f[4]))
            elif f[0] == "static": L["statics"].append(dict(ti=f[2], name=f[3]))
            elif f[0] == "method":
                at, ft, vo, nm = f[1], f[2], f[3], f[4]
                L["methods"].append(dict(name=nm, ft=ft, mprop=(at >> 2) & 7, access=at & 3, vo=vo,
                                         compgen=bool(at & 0x100)))   # CV_fldattr_t: access 0-1, mprop 2-4, compgenx 8
            elif f[0] == "vfptr": L["vfptr"] = True
            elif f[0] == "enumerator": L["enumerators"].append((f[3], f[2]))
        s.layout_cache[ti] = L; return L
    def argkey(s, ft):
        r, a, st, fa, th = s.sig(ft); return ",".join(s.decl(x) for x in a)
    def vtable(s, ti):
        """Primary vftable (the vfptr at offset 0): base's slots, then this class's introducing virtuals placed by
        their vbaseoff (byte offset in the table, CV mprop intro/pureintro), overrides replace the slot owner."""
        ti = s.full(ti)
        if ti in s.vt_cache: return s.vt_cache[ti]
        L = s.layout(ti); vt = {}
        prim = next((b for b in L["bases"] if b["off"] == 0 and s.has_vfptr(b["ti"])), None)
        if prim: vt = {k: dict(v, over=False) for k, v in s.vtable(prim["ti"]).items()}
        for md in L["methods"]:
            if md["mprop"] in (4, 6) and md["vo"] is not None:
                vt[md["vo"] // 8] = dict(name=md["name"], owner=L["name"], key=s.argkey(md["ft"]), ft=md["ft"],
                                         pure=md["mprop"] == 6, over=md["vo"] // 8 in vt,   # e.g. __vecDelDtor re-introduced
                                         intro=L["name"], introft=md["ft"])   # the class that declared the slot (kept by overrides)
        for md in L["methods"]:
            if md["mprop"] in (1, 5):   # virtual override
                key = s.argkey(md["ft"])
                cand = [k for k, v in vt.items() if v["name"] == md["name"] and v["key"] == key] or \
                       [k for k, v in vt.items() if v["name"] == md["name"]] or \
                       ([k for k, v in vt.items() if v["name"].startswith("~")] if md["name"].startswith("~") else [])
                if len(cand) >= 1:
                    k = cand[0]; vt[k] = dict(vt[k], name=md["name"], owner=L["name"], ft=md["ft"], pure=md["mprop"] == 5, over=True)
        s.vt_cache[ti] = vt; return vt
    def has_vfptr(s, ti):
        L = s.layout(ti)
        return L["vfptr"] or any(b["off"] == 0 and s.has_vfptr(b["ti"]) for b in L["bases"])

# ---------------------------------------------------------------- JSON type refs for Ghidra
def tref(T, ti, depth=0, need=None):
    """Compact type description: {"p":prim,"z":size} | {"s":name} | {"e":name,"z":size} | {"ptr":ref} |
    {"arr":ref,"n":count} | {"fn":sig}. By-value UDTs are added to `need` so their layouts get emitted too."""
    if ti < 0x1000:
        nm, z = SIMPLE.get(ti & 0xFF, ("undefined", 0))
        if (ti >> 8) & 7: return {"ptr": {"p": nm, "z": z}}
        return {"p": nm, "z": z}
    k, p, e = T.t.rec(ti); m = T.t.mm
    if k == LF["MODIFIER"]: return tref(T, struct.unpack_from("<I", m, p)[0], depth, need)
    if k == LF["BITFIELD"]: return tref(T, struct.unpack_from("<I", m, p)[0], depth, need)
    if k == LF["POINTER"]:
        ref, attr = struct.unpack_from("<II", m, p); mode = (attr >> 5) & 7
        if mode in (2, 3): return {"p": "undefined", "z": T.size(ti)}
        rk = T.t.rec(ref)[0] if ref >= 0x1000 else None
        if rk in (LF["PROCEDURE"], LF["MFUNCTION"]): return {"ptr": {"fn": T.decl(ref, "(*)")}}
        return {"ptr": tref(T, ref, depth + 1, None)}   # pointee: no layout needed (placeholder ok)
    if k == LF["ARRAY"]:
        el = struct.unpack_from("<I", m, p)[0]; es = T.size(el) or 1
        return {"arr": tref(T, el, depth, need), "n": T.t.num(p + 8)[0] // es}
    if k in UDT or k == LF["UNION"]:
        f = T.full(ti); nm = T.t.udt(f)[1]
        if need is not None: need.add(f)
        return {"s": nm, "z": T.size(f)}
    if k == LF["ENUM"]:
        return {"e": T.t.udt(T.full(ti))[1], "z": T.size(ti)}
    if k in (LF["PROCEDURE"], LF["MFUNCTION"]): return {"fn": T.decl(ti)}
    return {"p": "undefined", "z": T.size(ti)}

def bitinfo(T, ti):
    if ti >= 0x1000:
        k, p, e = T.t.rec(ti)
        if k == LF["BITFIELD"]:
            base, ln, pos = struct.unpack_from("<IBB", T.t.mm, p)                                       # [CVI] lfBitfield: type, length, position
            return base, ln, pos
    return None

def jlayout(T, L, need):
    j = dict(name=L["name"], kind=L["kind"], size=L["size"], bases=[], fields=[])
    if L["kind"] == "enum":
        j["enumerators"] = L["enumerators"]; return j
    if T.has_vfptr(L["ti"]) and not any(b["off"] == 0 and T.has_vfptr(b["ti"]) for b in L["bases"]):
        j["fields"].append(dict(name="vfptr", off=0, t={"ptr": {"s": L["name"] + "_vtbl", "z": 0}}))
    for b in L["bases"]:
        f = T.full(b["ti"]); need.add(f)
        j["bases"].append(dict(name=T.t.udt(f)[1], off=b["off"], size=T.size(f)))
    for mbr in L["members"]:
        bi = bitinfo(T, mbr["ti"])
        fd = dict(name=mbr["name"], off=mbr["off"], t=tref(T, mbr["ti"], 0, need))
        if bi: fd["bits"] = [bi[1], bi[2]]
        j["fields"].append(fd)
    vt = T.vtable(L["ti"]) if T.has_vfptr(L["ti"]) else {}
    if vt: j["vtbl"] = [[k, v["name"], v["owner"]] for k, v in sorted(vt.items())]
    return j

# ---------------------------------------------------------------- header text
ACC = {1: "private", 2: "protected", 3: "public"}
def header(T, L, src):
    o = []
    kw = L["kind"]
    o.append(f"// {L['name']}  sizeof = {L['size']:#x} ({L['size']})")
    o.append(f"// PDB {kw} record TPI {L['ti']:#x}" + (f"; declared at {src}" if src else ""))
    o.append("// Generated locally by tools/local-import/native_cache.py from Mordhau-Win64-Shipping.pdb (build 702625635). Do not edit.")
    o.append("#pragma once\n")
    if kw == "enum":
        o.append(f"enum {L['name']} : {L['utype']}\n{{")
        for nm, v in L["enumerators"]: o.append(f"\t{nm} = {v},")
        o.append("};"); return "\n".join(o) + "\n"
    bases = ", ".join(f"public {b['name']} /* {b['off']:#x} */" for b in L["bases"])
    o.append(f"{'struct' if kw in ('struct', 'interface') else kw} {L['name']}" + (f" : {bases}" if bases else ""))
    o.append("{")
    vt = T.vtable(L["ti"]) if T.has_vfptr(L["ti"]) else {}
    if vt:
        own = sum(1 for v in vt.values() if v["owner"] == L["name"])
        o.append(f"\t// primary vftable: {len(vt)} slots ({own} declared or overridden here). [slot] byte offset: method  (owner)")
        for k, v in sorted(vt.items()):
            r, a, st, fa, th = T.sig(v["ft"])
            mark = "override " if v["owner"] == L["name"] and v["over"] else ("new " if v["owner"] == L["name"] else "")
            o.append(f"\t//   [{k:3d}] {k*8:#05x}: {mark}{T.decl(r, v['name'] + '(' + ', '.join(T.decl(x) for x in a) + ')')}"
                     f"{' = 0' if v['pure'] else ''}  ({v['owner']})")
    if L["vfptr"]: o.append(f"\t/* 0x0000 */ void** __vftable;")
    vb = [b["name"] for b in L["vbases"]]
    if vb: o.append(f"\t// virtual bases: {', '.join(vb)}")
    end = None; last = None
    for mbr in L["members"]:
        bi = bitinfo(T, mbr["ti"])
        if bi:
            o.append(f"\t/* {mbr['off']:#06x}:{bi[2]} */ {T.decl(bi[0], mbr['name'])} : {bi[1]};")
        else:
            if L["kind"] != "union" and end is not None and mbr["off"] > end:
                o.append(f"\t// gap {end:#x}..{mbr['off']:#x} ({mbr['off'] - end} bytes)")
            o.append(f"\t/* {mbr['off']:#06x} */ {T.decl(mbr['ti'], mbr['name'])};")
        end = mbr["off"] + T.size(bi[0] if bi else mbr["ti"])
    if L["kind"] != "union" and end is not None and end < L["size"]:
        o.append(f"\t// tail {end:#x}..{L['size']:#x} ({L['size'] - end} bytes)")
    for st in L["statics"]: o.append(f"\tstatic {T.decl(st['ti'], st['name'])};")
    meth = [md for md in L["methods"] if not md["compgen"]]
    if meth:
        o.append("\n\t// methods (declaration order, types from the PDB; v = virtual, s = static)")
        for md in meth:
            r, a, stc, fa, th = T.sig(md["ft"])
            tag = "v " if md["mprop"] in (1, 4, 5, 6) else ("s " if md["mprop"] == 2 else "  ")
            o.append(f"\t// {tag}{ACC.get(md['access'], '')}: {T.decl(r, md['name'] + '(' + ', '.join('...' if x == 0 else T.decl(x) for x in a) + ')')}")
    o.append(f"}}; // sizeof = {L['size']:#x}")
    return "\n".join(o) + "\n"

def fname(k):
    k = re.sub(r"[^A-Za-z0-9_]", "_", k)
    return k if len(k) <= 120 else k[:100] + "_" + hashlib.sha1(k.encode()).hexdigest()[:8]

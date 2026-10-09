# Kismet bytecode reader: a Python port of CUE4Parse's FKismetArchive.cs / KismetExpression.cs (UE 4.26 layout,
# FFieldPath with ResolvedOwner), tracking the in-memory statement index the way FKismetArchive.Index does
# (FName = 12 bytes in memory, object pointers 8, FFieldPath 8). Used because `mdx json` / `mdx decomp` run with
# ReadScriptData off, so no function bodies are in extract/.
# Kismet bytecode reader ported from CUE4Parse FKismetArchive.cs / KismetExpression.cs (UE4.26 layout);
# `mi` = in-memory statement index (what jump offsets / ubergraph entry points refer to)
import struct
from pkg import *
class K:
    def __init__(s,pkg,b): s.p=pkg; s.b=b; s.o=0; s.mi=0
    def rd(s,fmt,mem=None):
        n=struct.calcsize(fmt); v=struct.unpack_from('<'+fmt,s.b,s.o); s.o+=n; s.mi+= n if mem is None else mem; return v[0] if len(v)==1 else v
    def name(s):
        i=s.rd('i'); n=s.rd('i'); s.mi+=4
        v=s.p.names[i]; return v if n==0 else f"{v}_{n-1}"
    def pidx(s):
        v=s.rd('i'); s.mi+=4; return s.p.obj(v)
    def fieldpath(s):
        m=s.mi; c=s.rd('i'); path=[s.name() for _ in range(c)]
        if s.owner: s.rd('i')
        s.mi=m+8; return '.'.join(path) if path else 'None'
    def arr(s,end):
        out=[]
        while True:
            e=s.expr()
            if e[0]==end: return out
            out.append(e)
    def expr(s):
        at=s.mi; t=s.rd('B'); E=lambda *a:(t,)+a+(at,)
        if t in (0,1,2,0x48,0x6c): return E(s.fieldpath())
        if t==0x04: return E(s.expr())
        if t==0x06: return E(s.rd('I'))
        if t==0x07: off=s.rd('I'); return E(off,s.expr())
        if t==0x09: s.rd('H'); s.rd('B'); return E(s.expr())
        if t in (0x0B,0x15,0x16,0x17,0x25,0x26,0x27,0x28,0x2A,0x2D,0x30,0x32,0x3A,0x3C,0x3E,0x40,0x4A,0x4D,0x50,0x53,0x5A,0x5E,0x66): return E()
        if t==0x0C: return E(s.rd('i'))
        if t==0x0F: pp=s.fieldpath(); return E(pp,s.expr(),s.expr())
        if t in (0x14,0x43,0x44,0x5F,0x60): return E(s.expr(),s.expr())
        if t in (0x12,0x19,0x1A): o=s.expr(); off=s.rd('I'); rv=s.fieldpath(); return E(o,off,rv,s.expr())
        if t==0x11: pp=s.fieldpath(); return E(pp,s.rd('B'))
        if t in (0x13,0x2E,0x52,0x54,0x55): c=s.pidx(); return E(c,s.expr())
        if t==0x18: off=s.rd('I'); return E(off,s.expr())
        if t in (0x1B,0x45): n=s.name(); return E(n,s.arr(0x16))
        if t in (0x1C,0x46,0x68): f=s.pidx(); return E(f,s.arr(0x16))
        if t==0x1D: return E(s.rd('i'))
        if t==0x1E: return E(s.rd('f'))
        if t==0x1F:
            e=s.b.index(b'\0',s.o); v=s.b[s.o:e].decode('latin-1'); n=e-s.o+1; s.o+=n; s.mi+=n; return E(v)
        if t==0x20: return E(s.pidx())
        if t==0x21: return E(s.name())
        if t in (0x22,0x23,0x41): return E(s.rd('fff'))
        if t in (0x24,0x2C): return E(s.rd('B'))
        if t==0x29:
            lt=s.rd('B')
            if lt==0: return E('empty')
            if lt==1: return E(s.expr(),s.expr(),s.expr())
            if lt in (2,3): return E(s.expr())
            if lt==4: s.pidx(); return E(s.expr(),s.expr())
            raise Exception('text %d'%lt)
        if t==0x2B: return E(s.rd('10f'))
        if t==0x2F: st=s.pidx(); s.rd('i'); return E(st,s.arr(0x30))
        if t==0x31: a=s.expr(); return E(a,s.arr(0x32))
        if t==0x33: return E(s.fieldpath())
        if t==0x34:
            o=s.o
            while s.b[o:o+2]!=b'\0\0' or (o-s.o)%2: o+=1
            v=s.b[s.o:o].decode('utf-16-le'); n=o-s.o+2; s.o+=n; s.mi+=n; return E(v)
        if t==0x35: return E(s.rd('q'))
        if t==0x36: return E(s.rd('Q'))
        if t==0x37: return E(s.rd('d'))
        if t==0x38: c=s.rd('B'); return E(c,s.expr())
        if t in (0x39,0x3B): a=s.expr(); s.rd('i'); return E(a,s.arr(0x3A if t==0x39 else 0x3C))
        if t in (0x3D,0x3F,0x65): pp=s.fieldpath(); s.rd('i'); return E(pp,s.arr({0x3D:0x3E,0x3F:0x40,0x65:0x66}[t]))
        if t==0x42: pp=s.fieldpath(); return E(pp,s.expr())
        if t==0x4B: return E(s.name())
        if t==0x4C: return E(s.rd('I'))
        if t==0x4E: return E(s.expr())
        if t==0x4F: return E(s.expr())
        if t==0x51: return E(s.expr())
        if t==0x5B: return E(s.rd('I'))
        if t in (0x5C,0x62): return E(s.expr(),s.expr())
        if t==0x5D: return E(s.expr())
        if t==0x61: n=s.name(); return E(n,s.expr(),s.expr())
        if t==0x63: s.pidx(); d=s.expr(); return E(d,s.arr(0x16))   # EX_CallMulticastDelegate: StackNode (UFunction*) first (CUE4Parse EX_CallMulticastDelegate)
        if t==0x64: pp=s.fieldpath(); return E(pp,s.expr())
        if t==0x67: return E(s.expr())
        if t==0x69:
            n=s.rd('H'); end=s.rd('I'); idx=s.expr(); cs=[]
            for _ in range(n): v=s.expr(); s.rd('I'); cs.append((v,s.expr()))
            return E(end,idx,cs,s.expr())
        if t==0x6A:
            et=s.rd('B')
            if et==4: return E(s.name())
            return E(et)
        if t==0x6B: return E(s.expr(),s.expr())
        if t==0x6D: return E(s.expr())
        raise Exception('unknown token %x at %d/%d'%(t,s.o,at))
def script(pkg,e,owner=True):
    b=pkg.data[e['off']:e['off']+e['size']]
    end=len(b)-12
    for S in range(1,end):
        st=end-S
        if struct.unpack_from('<i',b,st-4)[0]==S and b[end-1]==0x53:
            k=K(pkg,b[st:end]); k.owner=owner; out=[]
            try:
                while k.o<len(k.b): out.append(k.expr())
                return out
            except Exception:
                continue
    end=len(b)-14   # FUNC_Net: + RepOffset
    raise Exception('no script in '+e['name'])

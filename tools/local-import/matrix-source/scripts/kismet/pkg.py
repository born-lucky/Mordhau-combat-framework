# minimal UE4.26 cooked package reader: names, imports, exports (FPackageFileSummary / FObjectExport layouts)
import struct
class R:
    def __init__(s,b,o=0): s.b=b; s.o=o
    def i32(s): v=struct.unpack_from('<i',s.b,s.o)[0]; s.o+=4; return v
    def u32(s): v=struct.unpack_from('<I',s.b,s.o)[0]; s.o+=4; return v
    def i64(s): v=struct.unpack_from('<q',s.b,s.o)[0]; s.o+=8; return v
    def u16(s): v=struct.unpack_from('<H',s.b,s.o)[0]; s.o+=2; return v
    def u8(s): v=s.b[s.o]; s.o+=1; return v
    def f32(s): v=struct.unpack_from('<f',s.b,s.o)[0]; s.o+=4; return v
    def raw(s,n): v=s.b[s.o:s.o+n]; s.o+=n; return v
    def fstr(s):
        n=s.i32()
        if n==0: return ''
        if n<0: v=s.raw(-n*2).decode('utf-16-le')[:-1]
        else: v=s.raw(n)[:-1].decode('latin-1')
        return v
class Pkg:
    def __init__(s,uasset,uexp):
        h=open(uasset,'rb').read(); s.data=h+open(uexp,'rb').read(); s.hlen=len(h)
        r=R(h); assert r.u32()==0x9E2A83C1
        _leg=r.i32(); r.i32(); s.ver=r.i32(); r.i32()
        for _ in range(r.i32()): r.raw(20)
        s.total=r.i32(); r.fstr(); s.flags=r.u32()
        nc=r.i32(); no=r.i32()
        if not (s.flags & 0x80000000): r.fstr()   # LocalizationId unless PKG_FilterEditorOnly
        r.i32(); r.i32()
        ec=r.i32(); eo=r.i32(); ic=r.i32(); io=r.i32()
        r.o=no; s.names=[]
        for _ in range(nc): s.names.append(r.fstr()); r.u32()
        r.o=io; s.imports=[]
        for _ in range(ic):
            _cp=s.fname(r); cn=s.fname(r); outer=r.i32(); on=s.fname(r); s.imports.append((cn,on,outer))
        r.o=eo; s.exports=[]
        for _ in range(ec):
            cls=r.i32(); sup=r.i32(); _tmpl=r.i32(); outer=r.i32(); nm=s.fname(r); _fl=r.u32(); size=r.i64(); off=r.i64()
            r.raw(12); r.raw(16); r.u32(); r.raw(8); r.raw(4*5)
            s.exports.append(dict(cls=cls,sup=sup,outer=outer,name=nm,size=size,off=off))
    def fname(s,r):
        i=r.i32(); n=r.i32(); v=s.names[i]
        return v if n==0 else f"{v}_{n-1}"
    def obj(s,idx):
        if idx==0: return 'None'
        if idx<0: return s.imports[-idx-1][1]
        return s.exports[idx-1]['name']
    def cls_name(s,e): return s.obj(e['cls'])

"""Typed local-only defaults decoded from actual PDB layouts and literal replay bytes."""
import struct

class Layouts:
    def __init__(self, codeview, tpi):
        self.codeview=codeview;self.tpi=tpi;self.types=codeview.Types(tpi)
        self.byname={};self.skipped_fields=[]
        for ti in tpi.complete.values():self.byname.setdefault(tpi.udt(ti)[1],[]).append(ti)
    def layout(self,name):
        rows=self.byname.get(name,[])
        if len(rows)!=1:raise ValueError('Missing/ambiguous original complete type '+name)
        return self.types.layout(rows[0])
    def array_offsets(self,name,at=0,active=()):
        if name in active:raise ValueError('Recursive by-value PDB type')
        result=set();layout=self.layout(name)
        for base in layout['bases']:result.update(self.array_offsets(base['name'],at+base['off'],(*active,name)))
        for f in layout['members']:
            typ=self.types.decl(f['ti']);o=at+f['off']
            if typ.startswith('TArray<float'):result.add(o)
            elif typ.startswith('F') and '*' not in typ and '<' not in typ and typ in self.byname:
                result.update(self.array_offsets(typ,o,(*active,name)))
        return result
    def decode(self,name,raw,arrays,at=0,active=()):
        if name in active:raise ValueError('Recursive by-value PDB type')
        layout=self.layout(name);out={}
        for base in layout['bases']:out.update(self.decode(base['name'],raw,arrays,at+base['off'],(*active,name)))
        for f in layout['members']:
            typ=self.types.decl(f['ti']);o=at+f['off'];key=f['name'];size=self.types.size(f['ti'])
            if o<0 or o+size>len(raw):raise ValueError('PDB field outside reconstructed object')
            bits=self.codeview.bitinfo(self.types,f['ti'])
            if bits:
                value=int.from_bytes(raw[o:o+size],'little');value=(value>>bits[2])&((1<<bits[1])-1)
                out[key]=bool(value) if self.types.decl(bits[0])=='bool' else value
            elif typ=='float':out[key]=struct.unpack_from('<f',raw,o)[0]
            elif typ=='int32':out[key]=struct.unpack_from('<i',raw,o)[0]
            elif typ=='bool':out[key]=bool(raw[o])
            elif typ=='uint8' or (typ.startswith('E') and '*' not in typ and '<' not in typ):out[key]=int.from_bytes(raw[o:o+size],'little')
            elif typ.startswith('TArray<float'):out[key]=[struct.unpack('<f',v.to_bytes(4,'little'))[0] for v in arrays.get(o,[])]
            elif typ.startswith('F') and '*' not in typ and '<' not in typ and typ in self.byname:
                out[key]=self.decode(typ,raw,arrays,o,(*active,name))
            else:self.skipped_fields.append({'class':name,'field':key,'type':typ,'object_offset':o})
        return out
